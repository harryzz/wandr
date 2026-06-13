// IDrawingContextImpl → wasi:canvas. Transform handling: the canvas CTM
// is kept in sync with Avalonia's absolute Transform by concatenating the
// delta (pending × currentⁱⁿᵛ) — clips/layers then persist in the space
// they were pushed in, exactly matching Avalonia's semantics, with no
// per-draw save/restore pairs.
namespace WandrAvalonia;

using Avalonia;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Rendering.SceneGraph;
using GuestWorld.wit.imports.wasi.canvas.v0_0_2;

internal class WandrDrawingContext : IDrawingContextImpl
{
    /// CSS box-shadow blur radius → Gaussian sigma.
    private const float ShadowBlurToSigma = 0.5f;

    private Matrix _current = Matrix.Identity;
    private Matrix _pending = Matrix.Identity;
    private readonly Stack<Matrix> _saved = new();

    // No density scaling here: Avalonia renders 1:1 into a LOGICAL-sized
    // offscreen; FrameBridge upscales that to the physical buffer on
    // present. This keeps the canvas in a single coordinate space — no
    // per-frame scale, no save/restore games (which faulted under the
    // on-device aarch64 AOT) and no miniature second-pass artifact.
    private static IDraw.Canvas? Canvas => FrameBridge.CurrentCanvas;

    public Matrix Transform
    {
        get => _pending;
        set => _pending = value;
    }

    public RenderOptions RenderOptions { get; set; }

    public void Dispose() { }

    public object? GetFeature(Type t) => null;

    // ── transform sync ───────────────────────────────────────────────
    private void ApplyTransform()
    {
        if (_current == _pending || Canvas is null)
            return;
        var delta = _pending * _current.Invert();
        Canvas.Concat(ToTransform(delta));
        _current = _pending;
    }

    private static ITypes.Transform ToTransform(Matrix m) => new(
        (float)m.M11, (float)m.M21, (float)m.M31,
        (float)m.M12, (float)m.M22, (float)m.M32,
        (float)m.M13, (float)m.M23, (float)m.M33);

    private void PushState()
    {
        _saved.Push(_current);
    }

    private void PopState()
    {
        Canvas?.Restore();
        _current = _saved.Pop();
    }

    // ── paint building ───────────────────────────────────────────────
    private static uint ToColor(Color c, double opacity = 1.0)
    {
        var a = (uint)Math.Clamp(c.A * opacity, 0, 255);
        return (a << 24) | ((uint)c.R << 16) | ((uint)c.G << 8) | c.B;
    }

    private static Point Absolute(RelativePoint rp, Rect rect)
        => rp.Unit == RelativeUnit.Relative
            ? new Point(rect.X + rp.Point.X * rect.Width, rect.Y + rp.Point.Y * rect.Height)
            : rp.Point;

    private static ITypes.TileMode ToTile(GradientSpreadMethod spread) => spread switch
    {
        GradientSpreadMethod.Repeat => ITypes.TileMode.REPEAT,
        GradientSpreadMethod.Reflect => ITypes.TileMode.MIRROR,
        _ => ITypes.TileMode.CLAMP,
    };

    private static List<(float, uint)> ToStops(IGradientBrush brush)
    {
        var stops = new List<(float, uint)>(brush.GradientStops.Count);
        foreach (var stop in brush.GradientStops)
            stops.Add(((float)stop.Offset, ToColor(stop.Color)));
        return stops;
    }

    /// Resolves a brush to (color, shader). The caller disposes the shader
    /// after the draw call (paint carries a borrow).
    private static (uint color, ITypes.Shader? shader) ResolveBrush(IBrush brush, Rect target)
    {
        switch (brush)
        {
            case ISolidColorBrush solid:
                return (ToColor(solid.Color), null);
            case ILinearGradientBrush linear:
            {
                var start = Absolute(linear.StartPoint, target);
                var end = Absolute(linear.EndPoint, target);
                var shader = FrameBridge.Graphics.LinearGradient(
                    new ITypes.Point((float)start.X, (float)start.Y),
                    new ITypes.Point((float)end.X, (float)end.Y),
                    ToStops(linear), ToTile(linear.SpreadMethod), null);
                return (0xFFFFFFFF, shader);
            }
            case IRadialGradientBrush radial:
            {
                var center = Absolute(radial.Center, target);
                var radius = radial.Radius * Math.Max(target.Width, target.Height);
                var shader = FrameBridge.Graphics.RadialGradient(
                    new ITypes.Point((float)center.X, (float)center.Y),
                    (float)radius,
                    ToStops(radial), ToTile(radial.SpreadMethod), null);
                return (0xFFFFFFFF, shader);
            }
            case IConicGradientBrush conic:
            {
                var center = Absolute(conic.Center, target);
                var shader = FrameBridge.Graphics.SweepGradient(
                    new ITypes.Point((float)center.X, (float)center.Y),
                    (float)(conic.Angle - 90), (float)(conic.Angle + 270),
                    ToStops(conic), ITypes.TileMode.CLAMP, null);
                return (0xFFFFFFFF, shader);
            }
            default:
                FrameBridge.WarnOnce($"brush {brush.GetType().Name}");
                return (0xFF808080, null);
        }
    }

    private static ITypes.Paint MakePaint(ITypes.PaintStyle style, uint color, double opacity,
        ITypes.Shader? shader, float strokeWidth = 0,
        ITypes.StrokeCap cap = ITypes.StrokeCap.BUTT,
        ITypes.StrokeJoin join = ITypes.StrokeJoin.MITER,
        float miter = 10f, ITypes.MaskBlur? blur = null)
        => new(style, color, (byte)Math.Clamp(opacity * 255, 0, 255), ITypes.BlendMode.SRC_OVER,
               true, shader, strokeWidth, cap, join, miter, blur, null);

    private static (ITypes.Paint paint, ITypes.Shader? shader) FillPaint(IBrush brush, Rect target)
    {
        var (color, shader) = ResolveBrush(brush, target);
        return (MakePaint(ITypes.PaintStyle.FILL, color, brush.Opacity, shader), shader);
    }

    private static (ITypes.Paint paint, ITypes.Shader? shader)? StrokePaint(IPen? pen, Rect target)
    {
        if (pen?.Brush is null || pen.Thickness <= 0)
            return null;
        var (color, shader) = ResolveBrush(pen.Brush, target);
        var cap = pen.LineCap switch
        {
            PenLineCap.Round => ITypes.StrokeCap.ROUND,
            PenLineCap.Square => ITypes.StrokeCap.SQUARE,
            _ => ITypes.StrokeCap.BUTT,
        };
        var join = pen.LineJoin switch
        {
            PenLineJoin.Round => ITypes.StrokeJoin.ROUND,
            PenLineJoin.Bevel => ITypes.StrokeJoin.BEVEL,
            _ => ITypes.StrokeJoin.MITER,
        };
        return (MakePaint(ITypes.PaintStyle.STROKE, color, pen.Brush.Opacity, shader,
                          (float)pen.Thickness, cap, join, (float)pen.MiterLimit), shader);
    }

    private static ITypes.Rect ToRect(Rect r)
        => new((float)r.X, (float)r.Y, (float)r.Width, (float)r.Height);

    private static ITypes.RoundedRect ToRoundedRect(RoundedRect rr) => new(
        ToRect(rr.Rect),
        new ITypes.Point((float)rr.RadiiTopLeft.X, (float)rr.RadiiTopLeft.Y),
        new ITypes.Point((float)rr.RadiiTopRight.X, (float)rr.RadiiTopRight.Y),
        new ITypes.Point((float)rr.RadiiBottomRight.X, (float)rr.RadiiBottomRight.Y),
        new ITypes.Point((float)rr.RadiiBottomLeft.X, (float)rr.RadiiBottomLeft.Y));

    // ── drawing ──────────────────────────────────────────────────────
    public void Clear(Color color)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.Clear(ToColor(color));
    }

    public void DrawLine(IPen? pen, Point p1, Point p2)
    {
        if (Canvas is null) return;
        ApplyTransform();
        var bounds = new Rect(p1, p2).Normalize();
        if (StrokePaint(pen, bounds) is var sp && sp.HasValue)
        {
            Canvas.DrawLine(
                new ITypes.Point((float)p1.X, (float)p1.Y),
                new ITypes.Point((float)p2.X, (float)p2.Y),
                sp.Value.paint);
            sp.Value.shader?.Dispose();
        }
    }

    public void DrawRectangle(IBrush? brush, IPen? pen, RoundedRect rect, BoxShadows boxShadows = default)
    {
        if (Canvas is null) return;
        ApplyTransform();
        var target = rect.Rect;

        foreach (var shadow in boxShadows)
        {
            if (shadow.IsInset || shadow.Color.A == 0)
                continue;
            var srect = new RoundedRect(
                target.Translate(new Vector(shadow.OffsetX, shadow.OffsetY)).Inflate(shadow.Spread),
                rect.RadiiTopLeft, rect.RadiiTopRight, rect.RadiiBottomRight, rect.RadiiBottomLeft);
            var blur = shadow.Blur > 0
                ? new ITypes.MaskBlur(ITypes.BlurStyle.NORMAL, (float)(shadow.Blur * ShadowBlurToSigma))
                : (ITypes.MaskBlur?)null;
            var paint = MakePaint(ITypes.PaintStyle.FILL, ToColor(shadow.Color), 1.0, null, blur: blur);
            Canvas.DrawRoundedRect(ToRoundedRect(srect), paint);
        }

        if (brush is not null)
        {
            var (paint, shader) = FillPaint(brush, target);
            if (rect.IsRounded)
                Canvas.DrawRoundedRect(ToRoundedRect(rect), paint);
            else
                Canvas.DrawRect(ToRect(target), paint);
            shader?.Dispose();
        }

        if (StrokePaint(pen, target) is var sp && sp.HasValue)
        {
            if (rect.IsRounded)
                Canvas.DrawRoundedRect(ToRoundedRect(rect), sp.Value.paint);
            else
                Canvas.DrawRect(ToRect(target), sp.Value.paint);
            sp.Value.shader?.Dispose();
        }
    }

    public void DrawRectangle(IPen pen, Rect rect, float cornerRadius = 0)
        => DrawRectangle(null, pen,
            new RoundedRect(rect, cornerRadius, cornerRadius, cornerRadius, cornerRadius));

    public void DrawEllipse(IBrush? brush, IPen? pen, Rect rect)
    {
        if (Canvas is null) return;
        ApplyTransform();
        if (brush is not null)
        {
            var (paint, shader) = FillPaint(brush, rect);
            Canvas.DrawOval(ToRect(rect), paint);
            shader?.Dispose();
        }
        if (StrokePaint(pen, rect) is var sp && sp.HasValue)
        {
            Canvas.DrawOval(ToRect(rect), sp.Value.paint);
            sp.Value.shader?.Dispose();
        }
    }

    public void DrawGeometry(IBrush? brush, IPen? pen, IGeometryImpl geometry)
    {
        if (Canvas is null) return;
        ApplyTransform();

        var (path, rule, extraTransform) = geometry switch
        {
            WandrTransformedGeometry t => (((WandrGeometry)t.SourceGeometry).PathData,
                                           ((WandrGeometry)t.SourceGeometry).FillRule, (Matrix?)t.Transform),
            WandrGeometry g => (g.PathData, g.FillRule, null),
            _ => ("", FillRule.NonZero, null),
        };
        if (path.Length == 0)
            return;

        var wireRule = rule == FillRule.EvenOdd ? ITypes.FillRule.EVENODD : ITypes.FillRule.NONZERO;

        if (extraTransform is { } m)
        {
            Canvas.Save();
            Canvas.Concat(ToTransform(m));
        }

        var bounds = geometry.Bounds;
        if (brush is not null)
        {
            var (paint, shader) = FillPaint(brush, bounds);
            Canvas.DrawPath(path, wireRule, paint);
            shader?.Dispose();
        }
        if (StrokePaint(pen, bounds) is var sp && sp.HasValue)
        {
            Canvas.DrawPath(path, wireRule, sp.Value.paint);
            sp.Value.shader?.Dispose();
        }

        if (extraTransform is not null)
            Canvas.Restore();
    }

    public void DrawGlyphRun(IBrush? foreground, IGlyphRunImpl glyphRun)
    {
        if (Canvas is null || foreground is null || glyphRun is not WandrGlyphRun run)
            return;
        ApplyTransform();

        var typeface = (WandrGlyphTypeface)run.GlyphTypeface;
        var glyphs = new List<IGlyphs.PositionedGlyph>(run.GlyphInfos.Count);
        double penX = 0;
        foreach (var info in run.GlyphInfos)
        {
            glyphs.Add(new IGlyphs.PositionedGlyph(
                info.GlyphIndex,
                new ITypes.Point((float)(penX + info.GlyphOffset.X), (float)info.GlyphOffset.Y)));
            penX += info.GlyphAdvance;
        }

        var (paint, shader) = FillPaint(foreground, run.Bounds);
        GlyphsInterop.DrawGlyphs(
            Canvas, typeface.HostTypeface, (float)run.FontRenderingEmSize, glyphs,
            new ITypes.Point((float)run.BaselineOrigin.X, (float)run.BaselineOrigin.Y),
            paint);
        shader?.Dispose();
    }

    public void DrawBitmap(IBitmapImpl source, double opacity, Rect sourceRect, Rect destRect)
        => throw new NotSupportedException("wandr demo: bitmaps");

    public void DrawBitmap(IBitmapImpl source, IBrush opacityMask, Rect opacityMaskRect, Rect destRect)
        => throw new NotSupportedException("wandr demo: bitmaps");

    public void DrawRegion(IBrush? brush, IPen? pen, IPlatformRenderInterfaceRegion region)
        => throw new NotSupportedException("wandr demo: regions");

    public IDrawingContextLayerImpl CreateLayer(PixelSize size)
        => throw new NotSupportedException("wandr demo: intermediate layers");

    // ── state stack ──────────────────────────────────────────────────
    public void PushClip(Rect clip)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.Save();
        PushState();
        Canvas.ClipRect(ToRect(clip), true);
    }

    public void PushClip(RoundedRect clip)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.Save();
        PushState();
        Canvas.ClipRoundedRect(ToRoundedRect(clip), true);
    }

    public void PushClip(IPlatformRenderInterfaceRegion region)
        => throw new NotSupportedException("wandr demo: regions");

    public void PopClip() => PopState();

    public void PushGeometryClip(IGeometryImpl clip)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.Save();
        PushState();
        if (clip is WandrGeometry g && g.PathData.Length > 0)
        {
            Canvas.ClipPath(g.PathData,
                g.FillRule == FillRule.EvenOdd ? ITypes.FillRule.EVENODD : ITypes.FillRule.NONZERO,
                true);
        }
    }

    public void PopGeometryClip() => PopState();

    public void PushOpacity(double opacity, Rect? rect)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.SaveLayer(rect is { } r ? ToRect(r) : null,
                         (byte)Math.Clamp(opacity * 255, 0, 255));
        PushState();
    }

    public void PopOpacity() => PopState();

    public void PushLayer(Rect bounds)
    {
        if (Canvas is null) return;
        ApplyTransform();
        Canvas.SaveLayer(ToRect(bounds), 255);
        PushState();
    }

    public void PopLayer() => PopState();

    public void PushOpacityMask(IBrush mask, Rect bounds)
    {
        FrameBridge.WarnOnce("PushOpacityMask");
        if (Canvas is null) return;
        Canvas.Save();
        PushState();
    }

    public void PopOpacityMask() => PopState();

    public void PushBitmapBlendMode(BitmapBlendingMode blendingMode)
        => FrameBridge.WarnOnce("PushBitmapBlendMode");

    public void PopBitmapBlendMode() { }

    public void PushRenderOptions(RenderOptions renderOptions) { }

    public void PopRenderOptions() { }
}
