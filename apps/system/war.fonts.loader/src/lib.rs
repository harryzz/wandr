//! System-fonts loader component — task 41. Walks the host's
//! `/system/fonts/` directory (exposed to the dep via a WASI preopen
//! at `/system-fonts`) and returns font metadata + bytes.
//!
//! Filename parsing convention (Android system-fonts):
//!   NotoSerif-Regular.ttf      → family = "NotoSerif",      style = "Regular"
//!   NotoSerif-BoldItalic.ttf   → family = "NotoSerif",      style = "BoldItalic"
//!   DroidSansMono.ttf          → family = "DroidSansMono",  style = "Regular"
//!   NotoSansCJK-Regular.ttc    → family = "NotoSansCJK",    style = "Regular"
//!
//! Style is "Regular" by default; presence of a "-Suffix" in the
//! filename overrides. Extensions accepted: ttf, otf, ttc.

wit_bindgen::generate!({
    world: "loader-world",
    path: "../../../wit/system-fonts.wit",
});

use exports::war::fonts::loader::{FontInfo, Guest};
use std::fs;
use std::path::PathBuf;

/// Where the host preopens /system/fonts/ in the guest's WASI ctx.
const FONTS_ROOT: &str = "/system-fonts";

struct SystemFonts;

impl Guest for SystemFonts {
    fn list_all() -> Vec<FontInfo> {
        let mut out: Vec<FontInfo> = Vec::new();
        let entries = match fs::read_dir(FONTS_ROOT) {
            Ok(e) => e,
            Err(_) => return out, // preopen missing → empty list (host logs)
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(stem) = stem_if_font(name) else { continue };
            let (family, style) = parse_family_style(stem);
            out.push(FontInfo {
                family,
                style,
                path: format!("/system/fonts/{name}"),
            });
        }
        // Stable order: family ascending, then style.
        out.sort_by(|a, b| a.family.cmp(&b.family).then(a.style.cmp(&b.style)));
        out
    }

    fn load(family: String, style: String) -> Option<Vec<u8>> {
        // Try a few common filename patterns. Caller is expected to
        // have called list() first to know what's available, so this
        // is best-effort.
        let candidates = [
            format!("{FONTS_ROOT}/{family}-{style}.ttf"),
            format!("{FONTS_ROOT}/{family}-{style}.otf"),
            // "Regular" is the default — many font files drop it.
            format!("{FONTS_ROOT}/{family}.ttf"),
            format!("{FONTS_ROOT}/{family}.otf"),
        ];
        for p in &candidates {
            if let Ok(bytes) = fs::read(PathBuf::from(p)) {
                return Some(bytes);
            }
        }
        None
    }
}

export!(SystemFonts);

/// Returns the stem (without extension) if the file looks like a font.
fn stem_if_font(name: &str) -> Option<&str> {
    for ext in [".ttf", ".otf", ".ttc"] {
        if let Some(stem) = name.strip_suffix(ext) {
            return Some(stem);
        }
    }
    None
}

/// Parse "NotoSerif-BoldItalic" → ("NotoSerif", "BoldItalic"). Stem
/// without a "-" → ("DroidSansMono", "Regular").
fn parse_family_style(stem: &str) -> (String, String) {
    match stem.rsplit_once('-') {
        Some((fam, sty)) if !sty.is_empty() => (fam.to_string(), sty.to_string()),
        _ => (stem.to_string(), "Regular".to_string()),
    }
}
