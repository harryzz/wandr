//! Bulgarian Cyrillic keyboard language plugin (task 49 step 3).
//!
//! Exports `war:keyboard-lang/lang` — returns the БДС-style ЯВЕРТЫ rows
//! that previously lived as `ImeKeyboardDefaults.Bulgarian` inside
//! war.ime.keyboard. The IME loads this at startup via the generic dep
//! wiring (task 39) and merges the result into its 🌐 cycle.
//!
//! The plugin only supplies the language-specific character rows
//! (3 rows × variant) — the IME injects Shift / Backspace / Space /
//! Enter / 🌐 / Symbols-switch keys uniformly.

wit_bindgen::generate!({
    world: "lang-world",
    path:  "wit/keyboard-lang-bg.wit",
});

use exports::war::keyboard_lang_bg::lang::{Guest, Info, KeyDef, LayoutVariant};

struct WarLangBg;

impl Guest for WarLangBg {
    fn get_info() -> Info {
        Info {
            name:   "Български".to_string(),
            locale: "bg-BG".to_string(),
            is_rtl: false,
        }
    }

    fn get_layout(shifted: bool) -> LayoutVariant {
        let rows: &[&[&str]] = if shifted {
            &[
                &["Я","В","Е","Р","Т","Ъ","У","И","О","П"],
                &["А","С","Д","Ф","Г","Х","Й","К","Л"],
                &["З","Ь","Ц","Ж","Б","Н","М"],
            ]
        } else {
            &[
                &["я","в","е","р","т","ъ","у","и","о","п"],
                &["а","с","д","ф","г","х","й","к","л"],
                &["з","ь","ц","ж","б","н","м"],
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

export!(WarLangBg);
