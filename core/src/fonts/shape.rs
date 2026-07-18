//! rustybuzzによるテキストシェイピングとグリフ幅の取得。

use super::font::Font;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    /// 描画位置に対するアドバンス幅・オフセット(px)。
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    /// 全グリフのアドバンス幅の合計(px)。
    pub width: f32,
}

/// `font`で`text`を`font_size`(px)にシェイピングする。
pub fn shape_text(font: &Font, text: &str, font_size: f32) -> ShapedText {
    let face = font.face();
    let units_per_em = face.units_per_em() as f32;
    let scale = if units_per_em > 0.0 {
        font_size / units_per_em
    } else {
        0.0
    };

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    let output = rustybuzz::shape(&face, &[], buffer);

    let mut glyphs = Vec::with_capacity(output.len());
    let mut width = 0.0;
    for (info, pos) in output.glyph_infos().iter().zip(output.glyph_positions()) {
        let x_advance = pos.x_advance as f32 * scale;
        glyphs.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            x_advance,
            x_offset: pos.x_offset as f32 * scale,
            y_offset: pos.y_offset as f32 * scale,
        });
        width += x_advance;
    }

    ShapedText { glyphs, width }
}

/// レイアウトの行分割が必要とする、テキストの描画幅(px)のみを返す簡易API。
pub fn measure_text(font: &Font, text: &str, font_size: f32) -> f32 {
    shape_text(font, text, font_size).width
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_font() -> Font {
        Font::load(TEST_FONT_PATH).expect("should load bundled test font")
    }

    #[test]
    fn shapes_ascii_text_into_matching_glyph_count() {
        let font = test_font();
        let shaped = shape_text(&font, "Hi", 16.0);

        assert_eq!(shaped.glyphs.len(), 2);
        assert!(shaped.width > 0.0);
    }

    #[test]
    fn empty_text_produces_no_glyphs() {
        let font = test_font();
        let shaped = shape_text(&font, "", 16.0);

        assert!(shaped.glyphs.is_empty());
        assert_eq!(shaped.width, 0.0);
    }

    #[test]
    fn width_scales_linearly_with_font_size() {
        let font = test_font();
        let small = measure_text(&font, "Hello, world!", 10.0);
        let large = measure_text(&font, "Hello, world!", 20.0);

        assert!(
            (large - small * 2.0).abs() < 0.01,
            "width should scale linearly with font-size: small={small}, large={large}"
        );
    }

    #[test]
    fn longer_text_measures_wider() {
        let font = test_font();
        let short = measure_text(&font, "I", 16.0);
        let long = measure_text(&font, "Illustration", 16.0);

        assert!(long > short);
    }
}
