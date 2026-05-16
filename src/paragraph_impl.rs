use crate::bindings::my::skiko_gfx::paragraph::{Host, TextStyle as WitTextStyle};

impl Host for crate::HostState {
    fn create_paragraph_builder(&mut self, _width: f32) -> u32 {
        let style = skia_safe::textlayout::ParagraphStyle::new();
        let fc    = self.renderer.font_collection.clone();
        let builder = skia_safe::textlayout::ParagraphBuilder::new(&style, fc);
        let id = self.renderer.next_para_id;
        self.renderer.next_para_id += 1;
        self.renderer.para_builders.insert(id, builder);
        id
    }

    fn push_text_style(&mut self, id: u32, wit_style: WitTextStyle) {
        let Some(builder) = self.renderer.para_builders.get_mut(&id) else { return };
        let ts = wit_text_style_to_skia(&wit_style);
        builder.push_style(&ts);
    }

    fn add_text(&mut self, id: u32, text: Vec<u8>) {
        let Some(builder) = self.renderer.para_builders.get_mut(&id) else { return };
        let s = String::from_utf8_lossy(&text);
        builder.add_text(s.as_ref());
    }

    fn pop_text_style(&mut self, id: u32) {
        let Some(builder) = self.renderer.para_builders.get_mut(&id) else { return };
        builder.pop();
    }

    fn build_paragraph(&mut self, id: u32) -> u32 {
        let Some(builder) = self.renderer.para_builders.get_mut(&id) else { return 0 };
        let para = builder.build();
        let para_id = self.renderer.next_para_id;
        self.renderer.next_para_id += 1;
        self.renderer.paragraphs.insert(para_id, para);
        para_id
    }

    fn drop_paragraph_builder(&mut self, id: u32) {
        self.renderer.para_builders.remove(&id);
    }

    fn layout(&mut self, id: u32, width: f32) {
        if let Some(para) = self.renderer.paragraphs.get_mut(&id) {
            para.layout(width);
        }
    }

    fn paint_paragraph(&mut self, id: u32, x: f32, y: f32) {
        self.renderer.draw_paragraph(id, x, y);
    }

    fn get_height(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id)
            .map(|p| p.height())
            .unwrap_or(0.0)
    }

    fn get_line_count(&mut self, id: u32) -> u32 {
        self.renderer.paragraphs.get(&id)
            .map(|p| p.line_number() as u32)
            .unwrap_or(0)
    }

    fn get_max_width(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id).map(|p| p.max_width()).unwrap_or(0.0)
    }

    fn get_max_intrinsic_width(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id).map(|p| p.max_intrinsic_width()).unwrap_or(0.0)
    }

    fn get_min_intrinsic_width(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id).map(|p| p.min_intrinsic_width()).unwrap_or(0.0)
    }

    fn get_alphabetic_baseline(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id).map(|p| p.alphabetic_baseline()).unwrap_or(0.0)
    }

    fn get_ideographic_baseline(&mut self, id: u32) -> f32 {
        self.renderer.paragraphs.get(&id).map(|p| p.ideographic_baseline()).unwrap_or(0.0)
    }

    fn prepare_rects_for_range(
        &mut self, id: u32, start: u32, end: u32,
        height_mode: u32, width_mode: u32,
    ) -> u32 {
        use skia_safe::textlayout::{RectHeightStyle, RectWidthStyle};
        let height_style = match height_mode {
            0 => RectHeightStyle::Tight,
            1 => RectHeightStyle::Max,
            2 => RectHeightStyle::IncludeLineSpacingMiddle,
            3 => RectHeightStyle::IncludeLineSpacingTop,
            4 => RectHeightStyle::IncludeLineSpacingBottom,
            _ => RectHeightStyle::Strut,
        };
        let width_style = if width_mode == 1 {
            RectWidthStyle::Max
        } else {
            RectWidthStyle::Tight
        };
        let s = start as usize;
        let e = end as usize;
        let boxes = self.renderer.paragraphs.get(&id)
            .map(|p| p.get_rects_for_range(s..e, height_style, width_style))
            .unwrap_or_default();
        self.renderer.para_rect_cache = boxes;
        self.renderer.para_rect_cache.len() as u32
    }

    fn get_cached_rect_left(&mut self, index: u32) -> f32 {
        self.renderer.para_rect_cache.get(index as usize)
            .map(|b| b.rect.left).unwrap_or(0.0)
    }
    fn get_cached_rect_top(&mut self, index: u32) -> f32 {
        self.renderer.para_rect_cache.get(index as usize)
            .map(|b| b.rect.top).unwrap_or(0.0)
    }
    fn get_cached_rect_right(&mut self, index: u32) -> f32 {
        self.renderer.para_rect_cache.get(index as usize)
            .map(|b| b.rect.right).unwrap_or(0.0)
    }
    fn get_cached_rect_bottom(&mut self, index: u32) -> f32 {
        self.renderer.para_rect_cache.get(index as usize)
            .map(|b| b.rect.bottom).unwrap_or(0.0)
    }
    fn get_cached_rect_direction(&mut self, index: u32) -> u32 {
        use skia_safe::textlayout::TextDirection;
        self.renderer.para_rect_cache.get(index as usize)
            .map(|b| match b.direct { TextDirection::LTR => 0u32, TextDirection::RTL => 1u32 })
            .unwrap_or(0)
    }

    fn get_glyph_position_at_coordinate(&mut self, id: u32, x: f32, y: f32) -> u32 {
        self.renderer.paragraphs.get(&id)
            .map(|p| {
                let pa = p.get_glyph_position_at_coordinate((x, y));
                if pa.position < 0 { 0 } else { pa.position as u32 }
            })
            .unwrap_or(0)
    }

    fn get_word_boundary_start(&mut self, id: u32, offset: u32) -> u32 {
        self.renderer.paragraphs.get(&id)
            .map(|p| p.get_word_boundary(offset).start as u32)
            .unwrap_or(offset)
    }

    fn get_word_boundary_end(&mut self, id: u32, offset: u32) -> u32 {
        self.renderer.paragraphs.get(&id)
            .map(|p| p.get_word_boundary(offset).end as u32)
            .unwrap_or(offset)
    }

    fn drop_paragraph(&mut self, id: u32) {
        self.renderer.paragraphs.remove(&id);
    }
}

fn wit_text_style_to_skia(s: &WitTextStyle) -> skia_safe::textlayout::TextStyle {
    let weight = skia_safe::font_style::Weight::from(s.font_weight as i32);
    let slant  = if s.italic {
        skia_safe::font_style::Slant::Italic
    } else {
        skia_safe::font_style::Slant::Upright
    };
    let font_style = skia_safe::FontStyle::new(
        weight, skia_safe::font_style::Width::NORMAL, slant,
    );
    let c = s.color;
    let color = skia_safe::Color::from_argb(
        (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8,
    );
    let mut ts = skia_safe::textlayout::TextStyle::new();
    ts.set_font_size(s.font_size);
    ts.set_font_style(font_style);
    ts.set_color(color);
    let family = String::from_utf8_lossy(&s.font_family).into_owned();
    if !family.is_empty() {
        ts.set_font_families(&[family]);
    }
    ts
}
