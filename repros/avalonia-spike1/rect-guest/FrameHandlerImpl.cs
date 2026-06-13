// Task 106 / Avalonia spike #1 — bare C# reactor guest: one rect through
// wasi:canvas@0.0.2, driven by the host's frame-handler callbacks.
// Namespace must match wit-bindgen's generated export interop exactly.
namespace SpikeWorld.wit.exports.wasi.inputHandlers.v0_0_2;

using SpikeWorld.wit.imports.wasi.canvas.v0_0_2;

public class FrameHandlerImpl : IFrameHandler
{
    private static IEmbedding.CanvasContext? _context;

    public static void OnResize(uint width, uint height)
    {
        // Geometry is derived from the frame buffer each OnFrame, so the
        // resize only needs to trigger a redraw — which the host's frame
        // loop already does. Nothing to store.
    }

    public static void OnFrame(ulong nanos)
    {
        _context ??= EmbeddingInterop.GetContext();

        using var canvas = _context.GetCurrentBuffer();
        float surfaceW = canvas.Width();
        float surfaceH = canvas.Height();

        canvas.Clear(0xFF1E2530);

        // Centered rect, proportional to the surface (half width, quarter
        // height) — stays correct under live resize with no stored state.
        float w = surfaceW / 2f;
        float h = surfaceH / 4f;
        var rect = new ITypes.Rect((surfaceW - w) / 2f, (surfaceH - h) / 2f, w, h);
        var paint = new ITypes.Paint(
            ITypes.PaintStyle.FILL,
            0xFF4FC3F7,                 // non-premul ARGB
            255,
            ITypes.BlendMode.SRC_OVER,
            true,
            null,                       // no shader — solid color
            0f,
            ITypes.StrokeCap.BUTT,
            ITypes.StrokeJoin.MITER,
            4f,
            null,                       // no mask blur
            null);                      // no color filter
        canvas.DrawRect(rect, paint);

        _context.Present();
    }
}
