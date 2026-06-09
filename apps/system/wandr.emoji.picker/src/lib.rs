//! Emoji picker component — task 40. Implements `wandr:emoji/picker.list-all`
//! by returning a static curated emoji table. ~70 emojis across the
//! standard Unicode categories — enough for a visible grid UI without
//! shipping the entire Unicode emoji set.
//!
//! Companion to `markdown-renderer/` — same crate shape, same build
//! pipeline, same install path. Consumer is wandr-app's `EmojiCard`.
//!
//! See `tasks/40-emoji-picker.md`.

wit_bindgen::generate!({
    world: "picker-world",
    path: "../../../wit/emoji.wit",
});

use exports::wandr::emoji::picker::{Emoji, Guest};

struct EmojiPicker;

impl Guest for EmojiPicker {
    fn list_all() -> Vec<Emoji> {
        CURATED
            .iter()
            .map(|(glyph, name, category)| Emoji {
                glyph: (*glyph).to_string(),
                name: (*name).to_string(),
                category: (*category).to_string(),
            })
            .collect()
    }
}

export!(EmojiPicker);

/// Curated emoji table. Grouped by category, ordered for display.
/// (glyph, CLDR short name, category)
const CURATED: &[(&str, &str, &str)] = &[
    // ── Smileys & Emotion ──
    ("😀", "grinning face",              "Smileys & Emotion"),
    ("😁", "beaming face",               "Smileys & Emotion"),
    ("😂", "face with tears of joy",     "Smileys & Emotion"),
    ("🤣", "rolling on the floor",       "Smileys & Emotion"),
    ("😊", "smiling face",               "Smileys & Emotion"),
    ("😉", "winking face",               "Smileys & Emotion"),
    ("😎", "smiling face sunglasses",    "Smileys & Emotion"),
    ("🥳", "partying face",              "Smileys & Emotion"),
    ("😢", "crying face",                "Smileys & Emotion"),
    ("😡", "pouting face",               "Smileys & Emotion"),
    ("🤔", "thinking face",              "Smileys & Emotion"),
    ("😴", "sleeping face",              "Smileys & Emotion"),

    // ── People & Body ──
    ("👍", "thumbs up",                  "People & Body"),
    ("👎", "thumbs down",                "People & Body"),
    ("👏", "clapping hands",             "People & Body"),
    ("🙏", "folded hands",               "People & Body"),
    ("👋", "waving hand",                "People & Body"),
    ("✌", "victory hand",                "People & Body"),
    ("🤝", "handshake",                  "People & Body"),
    ("💪", "flexed biceps",              "People & Body"),

    // ── Animals & Nature ──
    ("🐶", "dog face",                   "Animals & Nature"),
    ("🐱", "cat face",                   "Animals & Nature"),
    ("🐭", "mouse face",                 "Animals & Nature"),
    ("🐹", "hamster",                    "Animals & Nature"),
    ("🐰", "rabbit face",                "Animals & Nature"),
    ("🦊", "fox",                        "Animals & Nature"),
    ("🐻", "bear",                       "Animals & Nature"),
    ("🐼", "panda",                      "Animals & Nature"),
    ("🦁", "lion",                       "Animals & Nature"),
    ("🌳", "deciduous tree",             "Animals & Nature"),
    ("🌸", "cherry blossom",             "Animals & Nature"),
    ("🌈", "rainbow",                    "Animals & Nature"),

    // ── Food & Drink ──
    ("🍎", "red apple",                  "Food & Drink"),
    ("🍌", "banana",                     "Food & Drink"),
    ("🍕", "pizza",                      "Food & Drink"),
    ("🍔", "hamburger",                  "Food & Drink"),
    ("🍣", "sushi",                      "Food & Drink"),
    ("🍩", "doughnut",                   "Food & Drink"),
    ("🍰", "shortcake",                  "Food & Drink"),
    ("☕", "hot beverage",                "Food & Drink"),
    ("🍺", "beer mug",                   "Food & Drink"),
    ("🍷", "wine glass",                 "Food & Drink"),

    // ── Activities ──
    ("⚽", "soccer ball",                 "Activities"),
    ("🏀", "basketball",                 "Activities"),
    ("🎾", "tennis",                     "Activities"),
    ("🏓", "ping pong",                  "Activities"),
    ("🎮", "video game",                 "Activities"),
    ("🎲", "game die",                   "Activities"),
    ("🎸", "guitar",                     "Activities"),
    ("🎨", "artist palette",             "Activities"),

    // ── Objects ──
    ("📱", "mobile phone",               "Objects"),
    ("💻", "laptop",                     "Objects"),
    ("⌨", "keyboard",                    "Objects"),
    ("🖥", "desktop computer",           "Objects"),
    ("📷", "camera",                     "Objects"),
    ("🔋", "battery",                    "Objects"),
    ("💡", "light bulb",                 "Objects"),
    ("🔑", "key",                        "Objects"),
    ("📚", "books",                      "Objects"),
    ("✏", "pencil",                      "Objects"),

    // ── Symbols ──
    ("❤", "red heart",                   "Symbols"),
    ("⭐", "star",                       "Symbols"),
    ("✅", "check mark",                 "Symbols"),
    ("❌", "cross mark",                 "Symbols"),
    ("⚠", "warning",                     "Symbols"),
    ("♻", "recycling symbol",            "Symbols"),
    ("🔥", "fire",                       "Symbols"),
    ("💯", "hundred points",             "Symbols"),

    // ── Flags ──
    ("🏳", "white flag",                 "Flags"),
    ("🏴", "black flag",                 "Flags"),
    ("🏁", "chequered flag",             "Flags"),
    ("🚩", "triangular flag",            "Flags"),
];
