// The host-side handles shared between the WIT exports (frame/input) and
// the Avalonia render interface. Avalonia's compositor renders
// INCREMENTALLY (dirty rects) and assumes the target retains previous
// contents; the embedder's frame buffer arrives cleared. So Avalonia
// draws into a persistent OFFSCREEN canvas (true retained semantics) and
// every present snapshots it onto the frame buffer.
namespace WandrAvalonia;

using GuestWorld.wit.imports.wasi.canvas.v0_0_2;
using GuestWorld.wit.imports.wandr.uiShell.v0_1_0;

internal static class FrameBridge
{
    private static IEmbedding.CanvasContext? _context;
    private static IDraw.Graphics? _graphics;

    internal static IEmbedding.CanvasContext Context =>
        _context ??= EmbeddingInterop.GetContext();

    internal static IDraw.Graphics Graphics =>
        _graphics ??= Context.Graphics();

    private static float _density;

    /// Display scale factor (dpi/160) — Avalonia's RenderScaling. The
    /// surface buffer is in PHYSICAL pixels, so without this everything
    /// lays out at 1 logical-px = 1 physical-px (unreadably small on a
    /// HiDPI panel). Queried once from the host; clamped ≥ 1.
    internal static float Density
    {
        get
        {
            if (_density == 0)
            {
                var d = MetricsInterop.GetDensity();
                _density = d >= 1f ? d : 1f;
            }
            return _density;
        }
    }

    private static IDraw.Canvas? _frame;     // per-frame presented buffer
    private static IDraw.Canvas? _retained;  // persistent compositor target

    /// What Avalonia's drawing context draws into — but ONLY during the
    /// on-frame pass, where the density base scale is active. Input events
    /// (on-pointer/on-key) make Avalonia render synchronously OUTSIDE that
    /// pass; those renders must no-op (canvas == null) or they draw
    /// UNSCALED into the retained canvas (the miniature-copy artifact).
    /// Deferring to on-frame is correct: input still updates state, and the
    /// next on-frame's incremental redraw reflects it.
    internal static IDraw.Canvas? CurrentCanvas => InFrame ? _retained : null;

    /// True only between BeginFrame and EndFrame.
    internal static bool InFrame { get; private set; }

    /// Set by the render target when the compositor actually draws a frame
    /// (it early-outs when nothing is dirty). Drives on-demand presenting:
    /// when idle we skip the buffer acquire + snapshot + blit + present
    /// entirely, so an idle UI costs ~nothing instead of a full redraw +
    /// full-surface copy every frame.
    private static bool _drawn;
    internal static void MarkDrawn() => _drawn = true;

    private static bool _sizeKnown;

    /// Physical surface pixels.
    internal static float SurfaceWidth { get; private set; }
    internal static float SurfaceHeight { get; private set; }

    /// Logical (density-independent) size Avalonia lays out in.
    internal static float LogicalWidth => SurfaceWidth / Density;
    internal static float LogicalHeight => SurfaceHeight / Density;

    /// The host's on-resize gives the surface size without touching the
    /// swapchain — so idle frames never need to acquire a buffer just to
    /// learn the size. Render frames re-confirm it from the real buffer.
    internal static void OnHostResize(uint width, uint height)
    {
        if (width == 0 || height == 0)
            return;
        SurfaceWidth = width;
        SurfaceHeight = height;
        _sizeKnown = true;
    }

    internal static void BeginFrame()
    {
        _drawn = false;

        // Learn the size once without a resize event (first frame): a single
        // buffer acquire, reused by EndFrame if this frame draws.
        if (!_sizeKnown)
        {
            _frame = Context.GetCurrentBuffer();
            SurfaceWidth = _frame.Width();
            SurfaceHeight = _frame.Height();
            _sizeKnown = true;
        }

        if (_retained is null ||
            _retained.Width() != SurfaceWidth || _retained.Height() != SurfaceHeight)
        {
            _retained?.Dispose();
            _retained = Graphics.NewOffscreen((uint)SurfaceWidth, (uint)SurfaceHeight);
        }

        // Density base: Avalonia renders in logical units (RenderScaling 1),
        // scale the retained canvas by density to map logical → physical.
        // Balanced by Restore in EndFrame.
        _retained!.Save();
        _retained.Scale(Density, Density);
        InFrame = true;
    }

    internal static void EndFrame()
    {
        InFrame = false;
        _retained!.Restore();

        if (!_drawn)
        {
            // Nothing changed this frame — don't acquire/snapshot/present.
            _frame?.Dispose();
            _frame = null;
            return;
        }

        _frame ??= Context.GetCurrentBuffer();
        using (var img = _retained.Snapshot())
        {
            var full = new ITypes.Rect(0, 0, SurfaceWidth, SurfaceHeight);
            _frame.DrawImageRect(img, full, full,
                new ITypes.Sampling(ITypes.FilterMode.NEAREST, ITypes.MipmapMode.NONE),
                new ITypes.Paint(ITypes.PaintStyle.FILL, 0xFFFFFFFF, 255,
                    ITypes.BlendMode.SRC, false, null, 0,
                    ITypes.StrokeCap.BUTT, ITypes.StrokeJoin.MITER, 4, null, null));
        }
        Context.Present();
        _frame.Dispose();
        _frame = null;
    }

    private static readonly HashSet<string> _warned = new();

    /// One-shot diagnostics for unimplemented paths — never silent, never
    /// spammy (feedback_canvas_stub_noop_traps: silent no-ops bite later).
    internal static void WarnOnce(string what)
    {
        if (_warned.Add(what))
        {
            try { Console.WriteLine($"avalonia-demo: {what}"); }
            catch { /* stdout is best-effort */ }
        }
    }
}
