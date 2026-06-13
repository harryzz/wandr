// Minimal harfbuzz C-API interop — just the shaping path Avalonia's
// ITextShaperImpl needs (blob → face → font → shape → infos/positions).
// DllImport("harfbuzz") is resolved statically via <DirectPInvoke> against
// native/libharfbuzz.a.
namespace TextGuest;

using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)]
internal struct HbGlyphInfo
{
    public uint Codepoint;   // glyph id after shaping
    public uint Mask;
    public uint Cluster;
    public uint Var1;
    public uint Var2;
}

[StructLayout(LayoutKind.Sequential)]
internal struct HbGlyphPosition
{
    public int XAdvance;
    public int YAdvance;
    public int XOffset;
    public int YOffset;
    public uint Var;
}

internal static unsafe class HarfBuzz
{
    internal const int HB_MEMORY_MODE_READONLY = 1;

    [DllImport("harfbuzz")] internal static extern nint hb_version_string();
    [DllImport("harfbuzz")] internal static extern nint hb_blob_create(byte* data, uint length, int mode, nint userData, nint destroy);
    [DllImport("harfbuzz")] internal static extern nint hb_face_create(nint blob, uint index);
    [DllImport("harfbuzz")] internal static extern uint hb_face_get_upem(nint face);
    [DllImport("harfbuzz")] internal static extern nint hb_font_create(nint face);
    [DllImport("harfbuzz")] internal static extern void hb_font_set_scale(nint font, int xScale, int yScale);
    [DllImport("harfbuzz")] internal static extern nint hb_buffer_create();
    [DllImport("harfbuzz")] internal static extern void hb_buffer_add_utf8(nint buffer, byte* text, int textLength, uint itemOffset, int itemLength);
    [DllImport("harfbuzz")] internal static extern void hb_buffer_guess_segment_properties(nint buffer);
    [DllImport("harfbuzz")] internal static extern void hb_shape(nint font, nint buffer, nint features, uint numFeatures);
    [DllImport("harfbuzz")] internal static extern HbGlyphInfo* hb_buffer_get_glyph_infos(nint buffer, out uint length);
    [DllImport("harfbuzz")] internal static extern HbGlyphPosition* hb_buffer_get_glyph_positions(nint buffer, out uint length);
    [DllImport("harfbuzz")] internal static extern void hb_buffer_destroy(nint buffer);
}
