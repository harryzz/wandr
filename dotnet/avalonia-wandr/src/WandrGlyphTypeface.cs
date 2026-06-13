// IGlyphTypeface over HarfBuzzSharp Face/Font built from raw font bytes —
// the Avalonia.Skia GlyphTypefaceImpl with the SKTypeface dependency
// replaced by harfbuzz itself (metrics via font extents + OpenTypeMetrics).
// Also owns the HOST typeface (glyphs.typeface.from-bytes over the same
// bytes) so shaper glyph ids and host rasterization always agree.
namespace WandrAvalonia;

using System.Runtime.InteropServices;
using Avalonia.Media;
using Avalonia.Platform;
using HarfBuzzSharp;
using GuestWorld.wit.imports.wasi.canvas.v0_0_2;

internal class WandrGlyphTypeface : IGlyphTypeface
{
    private readonly GCHandle _pin;

    public WandrGlyphTypeface(string familyName, FontWeight weight, FontStyle style,
                              byte[] fontBytes, uint index)
    {
        FamilyName = familyName;
        Weight = weight;
        Style = style;

        _pin = GCHandle.Alloc(fontBytes, GCHandleType.Pinned);
        var blob = new Blob(_pin.AddrOfPinnedObject(), fontBytes.Length, MemoryMode.ReadOnly);
        Face = new Face(blob, index);
        Font = new Font(Face);
        Font.SetFunctionsOpenType();

        HostTypeface = IGlyphs.Typeface.FromBytes(fontBytes, index);

        Font.TryGetHorizontalFontExtents(out var extents);
        Font.OpenTypeMetrics.TryGetPosition(OpenTypeMetricsTag.UnderlineOffset, out var underlineOffset);
        Font.OpenTypeMetrics.TryGetPosition(OpenTypeMetricsTag.UnderlineSize, out var underlineSize);
        Font.OpenTypeMetrics.TryGetPosition(OpenTypeMetricsTag.StrikeoutOffset, out var strikeoutOffset);
        Font.OpenTypeMetrics.TryGetPosition(OpenTypeMetricsTag.StrikeoutSize, out var strikeoutSize);

        // Avalonia convention (see Avalonia.Skia GlyphTypefaceImpl):
        // Ascent is negative-up, Descent positive-down, design units.
        Metrics = new FontMetrics
        {
            DesignEmHeight = (short)Face.UnitsPerEm,
            Ascent = -(int)extents.Ascender,
            Descent = -(int)extents.Descender,
            LineGap = (int)extents.LineGap,
            UnderlinePosition = -(int)underlineOffset,
            UnderlineThickness = (int)underlineSize,
            StrikethroughPosition = -(int)strikeoutOffset,
            StrikethroughThickness = (int)strikeoutSize,
            IsFixedPitch = false,
        };

        GlyphCount = Face.GlyphCount;
    }

    public Face Face { get; }
    public Font Font { get; }
    public IGlyphs.Typeface HostTypeface { get; }

    public string FamilyName { get; }
    public FontWeight Weight { get; }
    public FontStyle Style { get; }
    public FontStretch Stretch => FontStretch.Normal;
    public FontSimulations FontSimulations => FontSimulations.None;
    public FontMetrics Metrics { get; }
    public int GlyphCount { get; }

    public ushort GetGlyph(uint codepoint)
        => Font.TryGetGlyph(codepoint, out var glyph) ? (ushort)glyph : (ushort)0;

    public bool TryGetGlyph(uint codepoint, out ushort glyph)
    {
        glyph = GetGlyph(codepoint);
        return glyph != 0;
    }

    public ushort[] GetGlyphs(ReadOnlySpan<uint> codepoints)
    {
        var glyphs = new ushort[codepoints.Length];
        for (var i = 0; i < codepoints.Length; i++)
        {
            if (Font.TryGetGlyph(codepoints[i], out var glyph))
                glyphs[i] = (ushort)glyph;
        }
        return glyphs;
    }

    public int GetGlyphAdvance(ushort glyph) => Font.GetHorizontalGlyphAdvance(glyph);

    public int[] GetGlyphAdvances(ReadOnlySpan<ushort> glyphs)
    {
        var indices = new uint[glyphs.Length];
        for (var i = 0; i < glyphs.Length; i++)
            indices[i] = glyphs[i];
        return Font.GetHorizontalGlyphAdvances(indices);
    }

    public bool TryGetGlyphMetrics(ushort glyph, out GlyphMetrics metrics)
    {
        metrics = default;
        if (!Font.TryGetGlyphExtents(glyph, out var extents))
            return false;
        metrics = new GlyphMetrics
        {
            XBearing = extents.XBearing,
            YBearing = extents.YBearing,
            Width = extents.Width,
            Height = extents.Height,
        };
        return true;
    }

    public bool TryGetTable(uint tag, out byte[] table)
    {
        var blob = Face.ReferenceTable(tag);
        if (blob == null || blob.Length == 0)
        {
            table = Array.Empty<byte>();
            return false;
        }
        table = blob.AsSpan().ToArray();
        return true;
    }

    public void Dispose()
    {
        Font.Dispose();
        Face.Dispose();
        HostTypeface.Dispose();
        if (_pin.IsAllocated)
            _pin.Free();
    }
}
