//! Inline Formatting Context: 単純な貪欲法によるテキストの行分割と行ボックスの配置。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - `white-space: normal`相当の折り返し(連続する空白の畳み込み、単語単位の折り返し)
//!   のみ対応。長い単語1つで行幅を超える場合でも単語内では分割しない。同様に、
//!   1つの単語(空白で区切られたトークン)の途中でフォントが切り替わる場合でも、
//!   その単語自体は同じ行の中で分割しない(空白なしのCJK-Latin混在等での
//!   行中分割は非対応)
//! - インライン要素(`<b>`等)の境界はT5のボックスツリー構築時点でテキストに
//!   平坦化済みのため、行内での要素ごとの装飾(太字など)はM1では区別されない

use crate::fonts::{measure_text, shape_text, FontCollection, ShapedGlyph};
use crate::style::ComputedStyle;

use super::geometry::Rect;

/// 同一フォントで連続する区間(1単語の一部、または1単語全体)。
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// この区間の描画に使う、[`FontCollection`]内でのフォントのインデックス。
    pub font_index: usize,
    /// この区間の元テキスト(`ShapedGlyph::cluster`から文字を逆引きするために保持する。
    /// PDF出力の`/ToUnicode`CMap生成で使う)。
    pub text: String,
    pub glyphs: Vec<ShapedGlyph>,
    /// 行ボックス(`LineBox::rect`)の左端からの相対x座標。
    pub x_offset: f32,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    pub rect: Rect,
    pub runs: Vec<TextRun>,
}

/// `text`を`available_width`に収まるよう行分割し、`(origin_x, origin_y)`を起点に
/// 縦に積んだ行ボックス列を返す。1単語の中でフォントが切り替わる場合(CSSの
/// `font-family`フォールバック)は、その単語を複数の[`TextRun`]に分けてシェイピングする。
pub fn layout_inline_content(
    text: &str,
    style: &ComputedStyle,
    fonts: &FontCollection,
    available_width: f32,
    origin_x: f32,
    origin_y: f32,
) -> Vec<LineBox> {
    if fonts.is_empty() {
        return Vec::new();
    }

    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let font_size = style.font_size.0;
    let line_height = font_size * 1.2;
    let space_font_index = fonts.select_for_char(&style.font_family, ' ').unwrap_or(0);
    let space_width = measure_text(
        fonts
            .get(space_font_index)
            .expect("select_for_charは非空コレクションで有効な値を返す"),
        " ",
        font_size,
    );

    let mut lines = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width = 0.0f32;

    for word in words {
        let mut word_runs = split_word_into_font_runs(word, style, fonts);
        let word_width: f32 = word_runs.iter().map(|r| r.width).sum();

        if !current_runs.is_empty() && current_width + space_width + word_width > available_width {
            lines.push(finish_line(
                std::mem::take(&mut current_runs),
                current_width,
                origin_x,
                origin_y + lines.len() as f32 * line_height,
                line_height,
            ));
            current_width = 0.0;
        }

        if !current_runs.is_empty() {
            current_width += space_width;
        }

        for run in &mut word_runs {
            run.x_offset = current_width;
            current_width += run.width;
        }
        current_runs.extend(word_runs);
    }

    if !current_runs.is_empty() {
        lines.push(finish_line(
            current_runs,
            current_width,
            origin_x,
            origin_y + lines.len() as f32 * line_height,
            line_height,
        ));
    }

    lines
}

/// 単語を、文字ごとに選ばれたフォントが連続する区間ごとに[`TextRun`]へ分割する。
fn split_word_into_font_runs(
    word: &str,
    style: &ComputedStyle,
    fonts: &FontCollection,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current_font = None;
    let mut current_start = 0;

    for (byte_idx, c) in word.char_indices() {
        let font_index = fonts.select_for_char(&style.font_family, c).unwrap_or(0);
        match current_font {
            Some(f) if f == font_index => {}
            Some(f) => {
                runs.push(shape_run(
                    &word[current_start..byte_idx],
                    f,
                    fonts,
                    style.font_size.0,
                ));
                current_start = byte_idx;
                current_font = Some(font_index);
            }
            None => current_font = Some(font_index),
        }
    }
    if let Some(f) = current_font {
        runs.push(shape_run(
            &word[current_start..],
            f,
            fonts,
            style.font_size.0,
        ));
    }

    runs
}

fn shape_run(text: &str, font_index: usize, fonts: &FontCollection, font_size: f32) -> TextRun {
    let font = fonts.get(font_index).expect("font_indexは常に有効な範囲");
    let shaped = shape_text(font, text, font_size);
    TextRun {
        font_index,
        text: text.to_string(),
        glyphs: shaped.glyphs,
        x_offset: 0.0,
        width: shaped.width,
    }
}

fn finish_line(runs: Vec<TextRun>, width: f32, x: f32, y: f32, height: f32) -> LineBox {
    LineBox {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        runs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Font;

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn dejavu_only() -> FontCollection {
        FontCollection::new(vec![Font::load(DEJAVU_PATH).unwrap()])
    }

    fn dejavu_and_cjk() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).unwrap(),
            Font::load_indexed(CJK_PATH, 0).unwrap(),
        ])
    }

    #[test]
    fn empty_or_whitespace_only_text_produces_no_lines() {
        let style = ComputedStyle::default();
        let fonts = dejavu_only();
        assert!(layout_inline_content("", &style, &fonts, 200.0, 0.0, 0.0).is_empty());
        assert!(layout_inline_content("   \n\t  ", &style, &fonts, 200.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn empty_font_collection_produces_no_lines() {
        let style = ComputedStyle::default();
        let fonts = FontCollection::new(vec![]);
        assert!(layout_inline_content("hello", &style, &fonts, 200.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn text_that_fits_stays_on_a_single_line() {
        let style = ComputedStyle::default();
        let fonts = dejavu_only();
        let lines = layout_inline_content("hello world", &style, &fonts, 500.0, 10.0, 20.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].rect.x, 10.0);
        assert_eq!(lines[0].rect.y, 20.0);
        assert!(lines[0].rect.width > 0.0);
        assert_eq!(lines[0].rect.height, style.font_size.0 * 1.2);
        // "hello"と"world"それぞれ1ランクずつ、同じフォントで連続。
        assert_eq!(lines[0].runs.len(), 2);
        assert!(lines[0].runs.iter().all(|r| r.font_index == 0));
    }

    #[test]
    fn wraps_to_a_new_line_when_available_width_is_too_narrow() {
        let style = ComputedStyle::default();
        let fonts = dejavu_only();

        let one_line =
            layout_inline_content("hello world foo bar", &style, &fonts, 1000.0, 0.0, 0.0);
        assert_eq!(one_line.len(), 1);

        let wrapped = layout_inline_content("hello world foo bar", &style, &fonts, 60.0, 0.0, 0.0);
        assert!(wrapped.len() > 1);

        let line_height = style.font_size.0 * 1.2;
        assert_eq!(wrapped[1].rect.y, wrapped[0].rect.y + line_height);
    }

    #[test]
    fn overlong_single_word_is_not_split_and_still_placed() {
        let style = ComputedStyle::default();
        let fonts = dejavu_only();
        let lines = layout_inline_content(
            "supercalifragilisticexpialidocious",
            &style,
            &fonts,
            10.0,
            0.0,
            0.0,
        );

        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].rect.width > 10.0,
            "overflowing word should not be dropped or split"
        );
    }

    #[test]
    fn collapses_runs_of_whitespace_between_words() {
        let style = ComputedStyle::default();
        let fonts = dejavu_only();
        let lines = layout_inline_content("a    b\n\tc", &style, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        // 3単語、それぞれ1ランク。
        assert_eq!(lines[0].runs.len(), 3);
    }

    #[test]
    fn mixed_script_word_splits_into_separate_font_runs() {
        let style = ComputedStyle::default();
        let fonts = dejavu_and_cjk();

        // 空白なしでLatinとCJKが混在する1トークン。
        let lines = layout_inline_content("café日本語", &style, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].runs.len(),
            2,
            "should split into a Latin run and a CJK run"
        );
        assert_eq!(
            lines[0].runs[0].font_index, 0,
            "café should use DejaVu Sans"
        );
        assert_eq!(
            lines[0].runs[1].font_index, 1,
            "日本語 should use the CJK fallback font"
        );
        // 2つ目のランは1つ目の右側に続く。
        assert!(lines[0].runs[1].x_offset >= lines[0].runs[0].x_offset + lines[0].runs[0].width);
    }

    #[test]
    fn separate_cjk_and_latin_words_can_land_on_the_same_line() {
        let style = ComputedStyle::default();
        let fonts = dejavu_and_cjk();

        let lines = layout_inline_content("Invoice 請求書", &style, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2);
        assert_eq!(lines[0].runs[0].font_index, 0);
        assert_eq!(lines[0].runs[1].font_index, 1);
    }
}
