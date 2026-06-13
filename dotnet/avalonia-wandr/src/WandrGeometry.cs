// Geometry impls that record SVG path-data strings — wasi:canvas's
// draw-path/clip-path take the W3C path grammar directly, so a geometry
// IS its path string plus point-tracked bounds (the bounds/hit-test
// approximations follow Avalonia's own headless backend).
namespace WandrAvalonia;

using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.Text;
using Avalonia;
using Avalonia.Media;
using Avalonia.Platform;

internal class WandrGeometry : IGeometryImpl
{
    public WandrGeometry(string pathData, Rect bounds, FillRule fillRule = FillRule.NonZero)
    {
        PathData = pathData;
        Bounds = bounds;
        FillRule = fillRule;
    }

    public string PathData { get; protected set; }
    public FillRule FillRule { get; protected set; }
    public Rect Bounds { get; protected set; }
    public double ContourLength => 0;

    public virtual bool FillContains(Point point) => Bounds.Contains(point);

    public Rect GetRenderBounds(IPen? pen)
        => pen is null ? Bounds : Bounds.Inflate(pen.Thickness / 2);

    public IGeometryImpl GetWidenedGeometry(IPen pen) => this;

    public bool StrokeContains(IPen? pen, Point point) => false;

    public IGeometryImpl Intersect(IGeometryImpl geometry)
        => Combine(GeometryCombineMode.Intersect, this, geometry);

    public ITransformedGeometryImpl WithTransform(Matrix transform)
        => new WandrTransformedGeometry(this, transform);

    public bool TryGetPointAtDistance(double distance, out Point point)
    {
        point = default;
        return false;
    }

    public bool TryGetPointAndTangentAtDistance(double distance, out Point point, out Point tangent)
    {
        point = default;
        tangent = default;
        return false;
    }

    public bool TryGetSegment(double startDistance, double stopDistance, bool startOnBeginFigure,
        [NotNullWhen(true)] out IGeometryImpl? segmentGeometry)
    {
        segmentGeometry = null;
        return false;
    }

    internal static string F(double v) => v.ToString("0.###", CultureInfo.InvariantCulture);

    internal static WandrGeometry Rectangle(Rect r)
        => new($"M{F(r.X)} {F(r.Y)}H{F(r.Right)}V{F(r.Bottom)}H{F(r.X)}Z", r);

    internal static WandrGeometry Ellipse(Rect r)
    {
        var rx = r.Width / 2;
        var ry = r.Height / 2;
        var cx = r.X + rx;
        var cy = r.Y + ry;
        return new(
            $"M{F(cx - rx)} {F(cy)}" +
            $"A{F(rx)} {F(ry)} 0 1 0 {F(cx + rx)} {F(cy)}" +
            $"A{F(rx)} {F(ry)} 0 1 0 {F(cx - rx)} {F(cy)}Z", r);
    }

    internal static WandrGeometry Line(Point p1, Point p2)
        => new($"M{F(p1.X)} {F(p1.Y)}L{F(p2.X)} {F(p2.Y)}",
               new Rect(new Point(Math.Min(p1.X, p2.X), Math.Min(p1.Y, p2.Y)),
                        new Point(Math.Max(p1.X, p2.X), Math.Max(p1.Y, p2.Y))));

    /// Host-side boolean op (graphics.combine-paths) — every shipped
    /// renderer has a 2D boolean kernel; guests don't re-ship one.
    internal static WandrGeometry Combine(GeometryCombineMode mode, IGeometryImpl a, IGeometryImpl b)
    {
        var pa = (a as WandrGeometry)?.PathData ?? "";
        var pb = (b as WandrGeometry)?.PathData ?? "";
        var op = mode switch
        {
            GeometryCombineMode.Intersect => GuestWorld.wit.imports.wasi.canvas.v0_0_2.IDraw.PathOp.INTERSECT,
            GeometryCombineMode.Xor => GuestWorld.wit.imports.wasi.canvas.v0_0_2.IDraw.PathOp.XOR,
            GeometryCombineMode.Exclude => GuestWorld.wit.imports.wasi.canvas.v0_0_2.IDraw.PathOp.DIFFERENCE,
            _ => GuestWorld.wit.imports.wasi.canvas.v0_0_2.IDraw.PathOp.UNION,
        };
        var combined = FrameBridge.Graphics.CombinePaths(pa, pb, op) ?? "";
        var bounds = mode == GeometryCombineMode.Intersect
            ? a.Bounds.Intersect(b.Bounds)
            : a.Bounds.Union(b.Bounds);
        return new WandrGeometry(combined, bounds);
    }
}

internal class WandrTransformedGeometry : WandrGeometry, ITransformedGeometryImpl
{
    public WandrTransformedGeometry(WandrGeometry source, Matrix transform)
        : base(source.PathData, source.Bounds.TransformToAABB(transform), source.FillRule)
    {
        if (source is WandrTransformedGeometry transformed)
        {
            SourceGeometry = transformed.SourceGeometry;
            Transform = transformed.Transform * transform;
        }
        else
        {
            SourceGeometry = source;
            Transform = transform;
        }
    }

    public IGeometryImpl SourceGeometry { get; }
    public Matrix Transform { get; }
}

internal class WandrStreamGeometry : WandrGeometry, IStreamGeometryImpl
{
    private readonly List<Point> _points = new();

    public WandrStreamGeometry() : base("", default) { }

    public IStreamGeometryImpl Clone() => new ClonedStreamGeometry(PathData, Bounds, FillRule);

    public IStreamGeometryContextImpl Open() => new Context(this);

    public override bool FillContains(Point point)
    {
        // Convex approximation (Avalonia's own headless hit-test).
        for (var i = 0; i < _points.Count; i++)
        {
            var a = _points[i];
            var b = _points[(i + 1) % _points.Count];
            var c = _points[(i + 2) % _points.Count];

            Vector v0 = c - a;
            Vector v1 = b - a;
            Vector v2 = point - a;

            var dot00 = v0 * v0;
            var dot01 = v0 * v1;
            var dot02 = v0 * v2;
            var dot11 = v1 * v1;
            var dot12 = v1 * v2;

            var invDenom = 1 / (dot00 * dot11 - dot01 * dot01);
            var u = (dot11 * dot02 - dot01 * dot12) * invDenom;
            var v = (dot00 * dot12 - dot01 * dot02) * invDenom;
            if (u >= 0 && v >= 0 && u + v < 1)
                return true;
        }
        return false;
    }

    private class ClonedStreamGeometry : WandrGeometry, IStreamGeometryImpl
    {
        public ClonedStreamGeometry(string pathData, Rect bounds, FillRule rule)
            : base(pathData, bounds, rule) { }

        public IStreamGeometryImpl Clone() => this;

        public IStreamGeometryContextImpl Open()
            => throw new NotSupportedException("reopening a cloned stream geometry");
    }

    private class Context : IStreamGeometryContextImpl
    {
        private readonly WandrStreamGeometry _owner;
        private readonly StringBuilder _path = new();

        public Context(WandrStreamGeometry owner) => _owner = owner;

        private void Track(Point p) => _owner._points.Add(p);

        public void BeginFigure(Point startPoint, bool isFilled = true)
        {
            _path.Append($"M{F(startPoint.X)} {F(startPoint.Y)}");
            Track(startPoint);
        }

        public void LineTo(Point point)
        {
            _path.Append($"L{F(point.X)} {F(point.Y)}");
            Track(point);
        }

        public void CubicBezierTo(Point p1, Point p2, Point p3)
        {
            _path.Append($"C{F(p1.X)} {F(p1.Y)} {F(p2.X)} {F(p2.Y)} {F(p3.X)} {F(p3.Y)}");
            Track(p1); Track(p2); Track(p3);
        }

        public void QuadraticBezierTo(Point control, Point endPoint)
        {
            _path.Append($"Q{F(control.X)} {F(control.Y)} {F(endPoint.X)} {F(endPoint.Y)}");
            Track(control); Track(endPoint);
        }

        public void ArcTo(Point point, Size size, double rotationAngle, bool isLargeArc,
            SweepDirection sweepDirection)
        {
            _path.Append(
                $"A{F(size.Width)} {F(size.Height)} {F(rotationAngle)} " +
                $"{(isLargeArc ? 1 : 0)} {(sweepDirection == SweepDirection.Clockwise ? 1 : 0)} " +
                $"{F(point.X)} {F(point.Y)}");
            Track(point);
        }

        public void EndFigure(bool isClosed)
        {
            if (isClosed)
                _path.Append('Z');
            Flush();
        }

        public void SetFillRule(FillRule fillRule) => _owner.FillRule = fillRule;

        public void Dispose() => Flush();

        private void Flush()
        {
            _owner.PathData = _path.ToString();
            _owner.Bounds = CalculateBounds();
        }

        private Rect CalculateBounds()
        {
            if (_owner._points.Count == 0)
                return default;
            double left = double.MaxValue, right = double.MinValue;
            double top = double.MaxValue, bottom = double.MinValue;
            foreach (var p in _owner._points)
            {
                left = Math.Min(p.X, left);
                right = Math.Max(p.X, right);
                top = Math.Min(p.Y, top);
                bottom = Math.Max(p.Y, bottom);
            }
            return new Rect(new Point(left, top), new Point(right, bottom));
        }
    }
}
