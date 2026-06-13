// IPlatformRenderInterface for wandr — the WandrPlatformRenderInterface
// the feasibility memo planned: same shape as Avalonia's in-tree headless
// backend, but the drawing context forwards to wasi:canvas. Bitmaps and
// offscreen layers are out of scope for the demo and THROW (silent no-op
// stubs corrupt rendering much later — keep failures loud).
namespace WandrAvalonia;

using Avalonia;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Media.TextFormatting;
using Avalonia.Platform;

internal class WandrRenderInterface : IPlatformRenderInterface, IPlatformRenderInterfaceContext
{
    public static void Initialize()
    {
        AvaloniaLocator.CurrentMutable
            .Bind<IPlatformRenderInterface>().ToConstant(new WandrRenderInterface())
            .Bind<IFontManagerImpl>().ToConstant(new WandrFontManager())
            .Bind<ITextShaperImpl>().ToConstant(new WandrTextShaper());
    }

    public IPlatformRenderInterfaceContext CreateBackendContext(IPlatformGraphicsContext? graphicsContext)
    {
        return this;
    }

    public bool SupportsIndividualRoundRects => true;
    public AlphaFormat DefaultAlphaFormat => AlphaFormat.Premul;
    public PixelFormat DefaultPixelFormat => PixelFormat.Rgba8888;
    public bool IsSupportedBitmapPixelFormat(PixelFormat format) => false;
    public bool SupportsRegions => false;
    public IPlatformRenderInterfaceRegion CreateRegion() => throw new NotSupportedException("regions");

    public bool IsLost => false;
    public IReadOnlyDictionary<Type, object> PublicFeatures { get; } = new Dictionary<Type, object>();
    public object? TryGetFeature(Type featureType) => null;
    public void Dispose() { }

    // ── geometry ─────────────────────────────────────────────────────
    public IGeometryImpl CreateEllipseGeometry(Rect rect) => WandrGeometry.Ellipse(rect);
    public IGeometryImpl CreateLineGeometry(Point p1, Point p2) => WandrGeometry.Line(p1, p2);
    public IGeometryImpl CreateRectangleGeometry(Rect rect) => WandrGeometry.Rectangle(rect);
    public IStreamGeometryImpl CreateStreamGeometry() => new WandrStreamGeometry();

    public IGeometryImpl CreateGeometryGroup(FillRule fillRule, IReadOnlyList<IGeometryImpl> children)
    {
        var path = string.Concat(children.Select(c => (c as WandrGeometry)?.PathData ?? ""));
        var bounds = children.Count != 0
            ? children.Select(c => c.Bounds).Aggregate((a, b) => a.Union(b))
            : default;
        return new WandrGeometry(path, bounds, fillRule);
    }

    public IGeometryImpl CreateCombinedGeometry(GeometryCombineMode combineMode, IGeometryImpl g1, IGeometryImpl g2)
        => WandrGeometry.Combine(combineMode, g1, g2);

    public IGeometryImpl BuildGlyphRunGeometry(GlyphRun glyphRun)
    {
        FrameBridge.WarnOnce("BuildGlyphRunGeometry (text-as-geometry)");
        return new WandrGeometry("", glyphRun.Bounds);
    }

    // ── render targets ───────────────────────────────────────────────
    public IRenderTarget CreateRenderTarget(IEnumerable<object> surfaces)
        => new WandrRenderTarget();

    public IDrawingContextLayerImpl CreateOffscreenRenderTarget(PixelSize pixelSize, double scaling)
        => throw new NotSupportedException("wandr demo: offscreen render targets not implemented");

    public IRenderTargetBitmapImpl CreateRenderTargetBitmap(PixelSize size, Vector dpi)
        => throw new NotSupportedException("wandr demo: render target bitmaps not implemented");

    public IWriteableBitmapImpl CreateWriteableBitmap(PixelSize size, Vector dpi, PixelFormat format, AlphaFormat alphaFormat)
        => throw new NotSupportedException("wandr demo: writeable bitmaps not implemented");

    // ── bitmaps (not in demo scope) ──────────────────────────────────
    public IBitmapImpl LoadBitmap(string fileName) => throw new NotSupportedException("wandr demo: bitmaps");
    public IBitmapImpl LoadBitmap(Stream stream) => throw new NotSupportedException("wandr demo: bitmaps");
    public IWriteableBitmapImpl LoadWriteableBitmapToWidth(Stream stream, int width,
        BitmapInterpolationMode interpolationMode = BitmapInterpolationMode.HighQuality)
        => throw new NotSupportedException("wandr demo: bitmaps");
    public IWriteableBitmapImpl LoadWriteableBitmapToHeight(Stream stream, int height,
        BitmapInterpolationMode interpolationMode = BitmapInterpolationMode.HighQuality)
        => throw new NotSupportedException("wandr demo: bitmaps");
    public IWriteableBitmapImpl LoadWriteableBitmap(string fileName) => throw new NotSupportedException("wandr demo: bitmaps");
    public IWriteableBitmapImpl LoadWriteableBitmap(Stream stream) => throw new NotSupportedException("wandr demo: bitmaps");
    public IBitmapImpl LoadBitmap(PixelFormat format, AlphaFormat alphaFormat, IntPtr data, PixelSize size, Vector dpi, int stride)
        => throw new NotSupportedException("wandr demo: bitmaps");
    public IBitmapImpl LoadBitmapToWidth(Stream stream, int width,
        BitmapInterpolationMode interpolationMode = BitmapInterpolationMode.HighQuality)
        => throw new NotSupportedException("wandr demo: bitmaps");
    public IBitmapImpl LoadBitmapToHeight(Stream stream, int height,
        BitmapInterpolationMode interpolationMode = BitmapInterpolationMode.HighQuality)
        => throw new NotSupportedException("wandr demo: bitmaps");
    public IBitmapImpl ResizeBitmap(IBitmapImpl bitmapImpl, PixelSize destinationSize,
        BitmapInterpolationMode interpolationMode = BitmapInterpolationMode.HighQuality)
        => throw new NotSupportedException("wandr demo: bitmaps");

    // ── glyph runs ───────────────────────────────────────────────────
    public IGlyphRunImpl CreateGlyphRun(IGlyphTypeface glyphTypeface, double fontRenderingEmSize,
        IReadOnlyList<GlyphInfo> glyphInfos, Point baselineOrigin)
        => new WandrGlyphRun(glyphTypeface, fontRenderingEmSize, glyphInfos, baselineOrigin);
}

/// A glyph run keeps the shaped glyphs so DrawGlyphRun can forward the
/// exact ids/offsets to glyphs.draw-glyphs.
internal class WandrGlyphRun : IGlyphRunImpl
{
    public WandrGlyphRun(IGlyphTypeface glyphTypeface, double fontRenderingEmSize,
        IReadOnlyList<GlyphInfo> glyphInfos, Point baselineOrigin)
    {
        GlyphTypeface = glyphTypeface;
        FontRenderingEmSize = fontRenderingEmSize;
        GlyphInfos = glyphInfos;
        BaselineOrigin = baselineOrigin;

        double width = 0;
        foreach (var info in glyphInfos)
            width += info.GlyphAdvance;

        var scale = fontRenderingEmSize / glyphTypeface.Metrics.DesignEmHeight;
        var ascent = glyphTypeface.Metrics.Ascent * scale;   // negative-up
        var descent = glyphTypeface.Metrics.Descent * scale; // positive-down
        Bounds = new Rect(
            baselineOrigin.X,
            baselineOrigin.Y + ascent,
            width,
            descent - ascent);
    }

    public IReadOnlyList<GlyphInfo> GlyphInfos { get; }
    public Rect Bounds { get; }
    public Point BaselineOrigin { get; }
    public IGlyphTypeface GlyphTypeface { get; }
    public double FontRenderingEmSize { get; }

    public void Dispose() { }

    public IReadOnlyList<float> GetIntersections(float lowerBound, float upperBound)
        => Array.Empty<float>();
}

/// IRenderTarget2 with IsSuitableForDirectRendering, so the compositor
/// renders straight into our drawing context instead of demanding an
/// intermediate CreateLayer. Retention is honest: the context targets the
/// persistent offscreen canvas (FrameBridge), not the transient buffer.
internal class WandrRenderTarget : IRenderTarget2
{
    public RenderTargetProperties Properties => new()
    {
        RetainsPreviousFrameContents = true,
        IsSuitableForDirectRendering = true,
    };

    // A drawing context is created only when the compositor actually renders
    // (it early-outs when nothing is dirty) — so this is the on-demand
    // signal: tell FrameBridge a frame was drawn, otherwise it skips the
    // present entirely. The offscreen genuinely retains previous contents,
    // so incremental dirty-rect redraw is correct (and cheap when idle); the
    // mini artifact is handled by CurrentCanvas's InFrame gate, not by
    // forcing a full redraw.
    public IDrawingContextImpl CreateDrawingContext(bool scaleToDpi)
    {
        FrameBridge.MarkDrawn();
        return new WandrDrawingContext();
    }

    public IDrawingContextImpl CreateDrawingContext(PixelSize expectedPixelSize,
        out RenderTargetDrawingContextProperties properties)
    {
        FrameBridge.MarkDrawn();
        properties = new RenderTargetDrawingContextProperties
        {
            PreviousFrameIsRetained = true,
        };
        return new WandrDrawingContext();
    }

    public bool IsCorrupted => false;
    public void Dispose() { }
}
