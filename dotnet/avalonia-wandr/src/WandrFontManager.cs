// IFontManagerImpl over the host's /system-fonts preopen. The host
// preopens its PLATFORM font dir there — Noto Sans on the desktop, Roboto
// on Android — so the available base-Latin file name differs per device.
// Discover by reading in preference order; never assume one name.
namespace WandrAvalonia;

using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using Avalonia.Media;
using Avalonia.Platform;

internal class WandrFontManager : IFontManagerImpl
{
    private const string FontsRoot = "/system-fonts";
    private const string Family = "Sans";

    private static readonly string[] RegularCandidates =
        { "NotoSans-Regular.ttf", "Roboto-Regular.ttf", "DroidSans.ttf" };
    private static readonly string[] BoldCandidates =
        { "NotoSans-Bold.ttf", "Roboto-Bold.ttf", "DroidSans-Bold.ttf" };

    private WandrGlyphTypeface? _regular;
    private WandrGlyphTypeface? _bold;

    private WandrGlyphTypeface Resolve(FontWeight weight)
    {
        if ((int)weight >= 600)
            // Bold may be absent (this device ships no Roboto-Bold) — fall
            // back to the regular set so text renders, just not emboldened.
            return _bold ??= new WandrGlyphTypeface(
                Family, FontWeight.Bold, FontStyle.Normal,
                ReadFirst(BoldCandidates, RegularCandidates), 0);
        return _regular ??= new WandrGlyphTypeface(
            Family, FontWeight.Normal, FontStyle.Normal,
            ReadFirst(RegularCandidates), 0);
    }

    /// First candidate that actually reads from /system-fonts, scanning
    /// each set in order — read-probing avoids depending on wasi stat.
    private static byte[] ReadFirst(params string[][] candidateSets)
    {
        foreach (var set in candidateSets)
            foreach (var name in set)
            {
                try { return File.ReadAllBytes(Path.Combine(FontsRoot, name)); }
                catch { /* not on this device — try the next */ }
            }
        throw new FileNotFoundException(
            $"no base sans font in {FontsRoot} (tried " +
            $"{string.Join(", ", candidateSets.SelectMany(s => s))})");
    }

    public string GetDefaultFontFamilyName() => Family;

    public string[] GetInstalledFontFamilyNames(bool checkForUpdates = false)
        => new[] { Family };

    public bool TryMatchCharacter(int codepoint, FontStyle fontStyle, FontWeight fontWeight,
        FontStretch fontStretch, CultureInfo? culture, out Typeface typeface)
    {
        var resolved = Resolve(fontWeight);
        if (resolved.GetGlyph((uint)codepoint) == 0)
        {
            typeface = default;
            return false;
        }
        typeface = new Typeface(Family, fontStyle, fontWeight, fontStretch);
        return true;
    }

    public bool TryCreateGlyphTypeface(string familyName, FontStyle style, FontWeight weight,
        FontStretch stretch, [NotNullWhen(true)] out IGlyphTypeface? glyphTypeface)
    {
        glyphTypeface = Resolve(weight);
        return true;
    }

    public bool TryCreateGlyphTypeface(Stream stream, FontSimulations fontSimulations,
        [NotNullWhen(true)] out IGlyphTypeface? glyphTypeface)
    {
        using var ms = new MemoryStream();
        stream.CopyTo(ms);
        glyphTypeface = new WandrGlyphTypeface(
            Family,
            (fontSimulations & FontSimulations.Bold) != 0 ? FontWeight.Bold : FontWeight.Normal,
            (fontSimulations & FontSimulations.Oblique) != 0 ? FontStyle.Italic : FontStyle.Normal,
            ms.ToArray(), 0);
        return true;
    }
}
