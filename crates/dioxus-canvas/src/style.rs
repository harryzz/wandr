//! The minimal CSS-ish subset: parse an element's style map into a
//! `taffy::Style` (layout) plus the paint properties the painter needs
//! (background, text colour, font size/weight/italic, corner radius).
//!
//! Deliberately NOT a browser: flexbox + box model + text + colour + rounded
//! corners only. No cascade/specificity, grid, floats, or absolute positioning.

use std::collections::BTreeMap;

use taffy::{
    AlignItems, Dimension, Display, FlexDirection, JustifyContent, LengthPercentage,
    LengthPercentageAuto, Rect, Size, Style,
};

pub type StyleMap = BTreeMap<String, String>;

/// Paint-relevant properties resolved from the style map. `color` /
/// `font_size` / `font_weight` / `italic` inherit down to text nodes; the
/// rest are per-element.
#[derive(Clone, Copy, Debug)]
pub struct PaintProps {
    /// Background ARGB, or 0 (none) — skip the fill.
    pub background: u32,
    /// Corner radius in px (0 = square).
    pub radius: f32,
    /// Inherited text colour ARGB.
    pub color: u32,
    pub font_size: f32,
    pub font_weight: u32,
    pub italic: bool,
}

impl Default for PaintProps {
    fn default() -> Self {
        PaintProps {
            background: 0,
            radius: 0.0,
            color: 0xFFFFFFFF,
            font_size: 16.0,
            font_weight: 400,
            italic: false,
        }
    }
}

/// Parse a length token: `"12px"`, `"12"`, `"50%"`. Percent returns a fraction
/// (taffy's convention). `None` for `"auto"` / unparseable.
fn parse_len(v: &str) -> Option<Len> {
    let v = v.trim();
    if v == "auto" {
        return Some(Len::Auto);
    }
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|n| Len::Percent(n / 100.0));
    }
    let num = v.strip_suffix("px").unwrap_or(v).trim();
    num.parse::<f32>().ok().map(Len::Length)
}

enum Len {
    Length(f32),
    Percent(f32),
    Auto,
}

impl Len {
    fn dim(&self) -> Dimension {
        match self {
            Len::Length(n) => Dimension::Length(*n),
            Len::Percent(p) => Dimension::Percent(*p),
            Len::Auto => Dimension::Auto,
        }
    }
    fn lp(&self) -> LengthPercentage {
        match self {
            Len::Length(n) => LengthPercentage::Length(*n),
            Len::Percent(p) => LengthPercentage::Percent(*p),
            Len::Auto => LengthPercentage::Length(0.0),
        }
    }
    fn lpa(&self) -> LengthPercentageAuto {
        match self {
            Len::Length(n) => LengthPercentageAuto::Length(*n),
            Len::Percent(p) => LengthPercentageAuto::Percent(*p),
            Len::Auto => LengthPercentageAuto::Auto,
        }
    }
}

fn px(map: &StyleMap, key: &str) -> Option<f32> {
    map.get(key).and_then(|v| parse_len(v)).and_then(|l| match l {
        Len::Length(n) => Some(n),
        Len::Percent(_) | Len::Auto => None,
    })
}

/// `"#RGB"`, `"#RRGGBB"`, `"#AARRGGBB"`, `"transparent"` → ARGB u32 (0 = none).
pub fn parse_color(v: &str) -> u32 {
    let v = v.trim();
    if v.eq_ignore_ascii_case("transparent") || v.eq_ignore_ascii_case("none") {
        return 0;
    }
    if let Some(hex) = v.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let r = dup(&hex[0..1]);
                let g = dup(&hex[1..2]);
                let b = dup(&hex[2..3]);
                0xFF00_0000 | (r << 16) | (g << 8) | b
            }
            6 => 0xFF00_0000 | u32::from_str_radix(hex, 16).unwrap_or(0),
            8 => u32::from_str_radix(hex, 16).unwrap_or(0),
            _ => 0,
        };
    }
    0
}

fn dup(s: &str) -> u32 {
    let n = u32::from_str_radix(s, 16).unwrap_or(0);
    (n << 4) | n
}

/// Build a `taffy::Style` from the element's style map.
pub fn to_taffy(map: &StyleMap) -> Style {
    let mut s = Style::DEFAULT;

    s.display = match map.get("display").map(String::as_str) {
        Some("none") => Display::None,
        Some("block") => Display::Block,
        _ => Display::Flex, // default to flex — the workhorse layout
    };

    s.flex_direction = match map.get("flex-direction").map(String::as_str) {
        Some("column") => FlexDirection::Column,
        Some("row-reverse") => FlexDirection::RowReverse,
        Some("column-reverse") => FlexDirection::ColumnReverse,
        _ => FlexDirection::Row,
    };

    if let Some(l) = map.get("width").and_then(|v| parse_len(v)) {
        s.size.width = l.dim();
    }
    if let Some(l) = map.get("height").and_then(|v| parse_len(v)) {
        s.size.height = l.dim();
    }

    if let Some(p) = px(map, "padding") {
        let lp = LengthPercentage::Length(p);
        s.padding = Rect { left: lp, right: lp, top: lp, bottom: lp };
    }
    if let Some(l) = map.get("padding-left").and_then(|v| parse_len(v)) {
        s.padding.left = l.lp();
    }
    if let Some(l) = map.get("padding-right").and_then(|v| parse_len(v)) {
        s.padding.right = l.lp();
    }
    if let Some(l) = map.get("padding-top").and_then(|v| parse_len(v)) {
        s.padding.top = l.lp();
    }
    if let Some(l) = map.get("padding-bottom").and_then(|v| parse_len(v)) {
        s.padding.bottom = l.lp();
    }

    if let Some(l) = map.get("margin").and_then(|v| parse_len(v)) {
        let m = l.lpa();
        s.margin = Rect { left: m, right: m, top: m, bottom: m };
    }

    if let Some(g) = px(map, "gap") {
        s.gap = Size { width: LengthPercentage::Length(g), height: LengthPercentage::Length(g) };
    }

    if let Some(v) = map.get("flex-grow").and_then(|v| v.trim().parse::<f32>().ok()) {
        s.flex_grow = v;
    }

    s.align_items = match map.get("align-items").map(String::as_str) {
        Some("center") => Some(AlignItems::Center),
        Some("flex-start") | Some("start") => Some(AlignItems::FlexStart),
        Some("flex-end") | Some("end") => Some(AlignItems::FlexEnd),
        Some("stretch") => Some(AlignItems::Stretch),
        _ => None,
    };

    s.justify_content = match map.get("justify-content").map(String::as_str) {
        Some("center") => Some(JustifyContent::Center),
        Some("flex-start") | Some("start") => Some(JustifyContent::FlexStart),
        Some("flex-end") | Some("end") => Some(JustifyContent::FlexEnd),
        Some("space-between") => Some(JustifyContent::SpaceBetween),
        Some("space-around") => Some(JustifyContent::SpaceAround),
        Some("space-evenly") => Some(JustifyContent::SpaceEvenly),
        _ => None,
    };

    s
}

/// Resolve paint properties, inheriting `color`/`font-*` from the parent.
pub fn to_paint(map: &StyleMap, inherited: &PaintProps) -> PaintProps {
    let mut p = *inherited;
    p.background = 0;
    p.radius = 0.0;
    if let Some(bg) = map.get("background").or_else(|| map.get("background-color")) {
        p.background = parse_color(bg);
    }
    if let Some(r) = px(map, "border-radius") {
        p.radius = r;
    }
    if let Some(c) = map.get("color") {
        p.color = parse_color(c);
    }
    if let Some(fs) = px(map, "font-size") {
        p.font_size = fs;
    }
    if let Some(w) = map.get("font-weight") {
        p.font_weight = match w.as_str() {
            "bold" => 700,
            "normal" => 400,
            n => n.parse().unwrap_or(400),
        };
    }
    if let Some(fst) = map.get("font-style") {
        p.italic = fst == "italic";
    }
    p
}
