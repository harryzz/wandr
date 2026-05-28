//! French AZERTY keyboard language plugin (task 49 step 4).
//!
//! Exports `war:keyboard-lang/lang` — returns the standard AZERTY
//! letter rows (with French-specific keys ù and à in the unshifted
//! variant; lone accent dead-keys deferred for now since `KeyDef`
//! doesn't carry the dead-key concept yet).
//!
//! Second concrete plugin alongside `war.lang.bg/`. Proves the
//! contract for a non-Cyrillic script + makes the 🌐 cycle a real
//! multi-step rotation (English → Bulgarian → French → …).
//!
//! The plugin only supplies the language-specific character rows —
//! the IME injects Shift / Backspace / Space / Enter / 🌐 / Symbols-
//! switch keys uniformly across all languages.

wit_bindgen::generate!({
    world: "lang-world",
    path:  "wit/keyboard-lang-fr.wit",
});

use exports::war::keyboard_lang_fr::lang::{Guest, Info, KeyDef, LayoutVariant};

struct WarLangFr;

impl Guest for WarLangFr {
    fn get_info() -> Info {
        Info {
            name:   "Français".to_string(),
            locale: "fr-FR".to_string(),
            is_rtl: false,
        }
    }

    fn get_layout(shifted: bool) -> LayoutVariant {
        let rows: &[&[&str]] = if shifted {
            &[
                &["A","Z","E","R","T","Y","U","I","O","P"],
                &["Q","S","D","F","G","H","J","K","L","M"],
                &["W","X","C","V","B","N"],
            ]
        } else {
            &[
                &["a","z","e","r","t","y","u","i","o","p"],
                &["q","s","d","f","g","h","j","k","l","m","ù"],
                &["w","x","c","v","b","n","à"],
            ]
        };
        LayoutVariant {
            rows: rows.iter().map(|r| r.iter().map(|g| text(g)).collect()).collect(),
        }
    }
}

fn text(glyph: &str) -> KeyDef {
    let code_point = glyph.chars().next().map(|c| c as u32).unwrap_or(0);
    KeyDef {
        display:    glyph.to_string(),
        code_point,
        key_id:     0,
        width:      1.0,
    }
}

export!(WarLangFr);
