// Task 107 / Avalonia spike #2 — shape once with statically-linked
// harfbuzz, draw every frame through wasi:canvas/glyphs. This is the
// Avalonia text model in miniature: guest-side shaper produces positioned
// glyph ids against the guest's own font bytes; the host rasterizes from
// those exact bytes (typeface.from-bytes contract).
namespace TextSpikeWorld.wit.exports.wasi.inputHandlers.v0_0_2;

using System.Runtime.InteropServices;
using System.Text;
using TextGuest;
using TextSpikeWorld.wit.imports.wasi.canvas.v0_0_2;

public class FrameHandlerImpl : IFrameHandler
{
    private const string FontPath = "/system-fonts/NotoSans-Regular.ttf";
    private const string Text = "Avalonia spike #2 — harfbuzz: AV fi ffl";
    // The one layout policy constant: the shaped line spans this fraction
    // of the surface width; em size is derived from it each frame.
    private const float TargetWidthFraction = 0.8f;

    private static IEmbedding.CanvasContext? _context;
    private static IGlyphs.Typeface? _typeface;

    // Shaping output, font units (font scale = upem).
    private record struct ShapedGlyph(uint Id, int XOffset, int YOffset, int XAdvance, int YAdvance);
    private static ShapedGlyph[]? _shaped;
    private static uint _upem;
    private static long _totalAdvance;

    public static void OnResize(uint width, uint height)
    {
        // Geometry and em size are re-derived from the buffer each frame.
    }

    private static unsafe void ShapeOnce()
    {
        if (_shaped != null) return;

        var fontBytes = File.ReadAllBytes(FontPath);
        _typeface = IGlyphs.Typeface.FromBytes(fontBytes, 0);

        // Keep the managed font bytes alive+pinned for the blob's lifetime
        // (process lifetime here — face/font are cached forever).
        var pin = GCHandle.Alloc(fontBytes, GCHandleType.Pinned);
        nint blob = HarfBuzz.hb_blob_create(
            (byte*)pin.AddrOfPinnedObject(), (uint)fontBytes.Length,
            HarfBuzz.HB_MEMORY_MODE_READONLY, 0, 0);
        nint face = HarfBuzz.hb_face_create(blob, 0);
        _upem = HarfBuzz.hb_face_get_upem(face);
        nint font = HarfBuzz.hb_font_create(face);
        // Shape in font units; canvas units = value * emSize / upem.
        HarfBuzz.hb_font_set_scale(font, (int)_upem, (int)_upem);

        nint buffer = HarfBuzz.hb_buffer_create();
        var utf8 = Encoding.UTF8.GetBytes(Text);
        fixed (byte* p = utf8)
        {
            HarfBuzz.hb_buffer_add_utf8(buffer, p, utf8.Length, 0, utf8.Length);
        }
        HarfBuzz.hb_buffer_guess_segment_properties(buffer);
        HarfBuzz.hb_shape(font, buffer, 0, 0);

        var infos = HarfBuzz.hb_buffer_get_glyph_infos(buffer, out uint count);
        var positions = HarfBuzz.hb_buffer_get_glyph_positions(buffer, out _);

        _shaped = new ShapedGlyph[count];
        _totalAdvance = 0;
        for (uint i = 0; i < count; i++)
        {
            _shaped[i] = new ShapedGlyph(
                infos[i].Codepoint,
                positions[i].XOffset, positions[i].YOffset,
                positions[i].XAdvance, positions[i].YAdvance);
            _totalAdvance += positions[i].XAdvance;
        }
        HarfBuzz.hb_buffer_destroy(buffer);

        try
        {
            Console.WriteLine(
                $"text-guest: harfbuzz {Marshal.PtrToStringUTF8(HarfBuzz.hb_version_string())} " +
                $"shaped {utf8.Length} utf8 bytes -> {count} glyphs (upem {_upem}, advance {_totalAdvance})");
        }
        catch
        {
            // stdout is best-effort evidence, never load-bearing
        }
    }

    public static void OnFrame(ulong nanos)
    {
        _context ??= EmbeddingInterop.GetContext();
        ShapeOnce();

        using var canvas = _context.GetCurrentBuffer();
        float surfaceW = canvas.Width();
        float surfaceH = canvas.Height();

        canvas.Clear(0xFF1E2530);

        // Em size derived so the line fills TargetWidthFraction of the
        // surface width at any resolution.
        float emSize = TargetWidthFraction * surfaceW * _upem / _totalAdvance;
        float s = emSize / _upem;

        var glyphs = new List<IGlyphs.PositionedGlyph>(_shaped!.Length);
        float penX = 0, penY = 0;
        foreach (var g in _shaped)
        {
            // hb y axis is up, canvas y is down.
            glyphs.Add(new IGlyphs.PositionedGlyph(
                g.Id,
                new ITypes.Point(penX + g.XOffset * s, penY - g.YOffset * s)));
            penX += g.XAdvance * s;
            penY -= g.YAdvance * s;
        }

        var origin = new ITypes.Point((surfaceW - (float)(_totalAdvance * s)) / 2f, surfaceH / 2f);
        var paint = new ITypes.Paint(
            ITypes.PaintStyle.FILL,
            0xFFE8EDF2,
            255,
            ITypes.BlendMode.SRC_OVER,
            true,
            null,
            0f,
            ITypes.StrokeCap.BUTT,
            ITypes.StrokeJoin.MITER,
            4f,
            null,
            null);
        GlyphsInterop.DrawGlyphs(canvas, _typeface!, emSize, glyphs, origin, paint);

        _context.Present();
    }
}
