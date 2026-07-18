//! Inline Formatting Context: 単純な貪欲法によるテキストの行分割と行ボックスの配置。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - `white-space: normal`相当の折り返し(連続する空白の畳み込み、単語単位の折り返し)
//!   のみ対応。長い単語1つで行幅を超える場合でも単語内では分割しない。同様に、
//!   1つの単語(空白で区切られたトークン)の途中でフォントやスタイルが切り替わる
//!   場合でも、その単語自体は同じ行の中で分割しない(空白なしのCJK-Latin混在等での
//!   行中分割は非対応)
//! - 単語間の空白の幅は、直前のテキストランのフォント・サイズを基準に測る
//!   (前後で大きくフォントサイズが異なる境界では厳密ではない)

use std::collections::HashMap;

use crate::fonts::{measure_text, shape_text, FontCollection, ShapedGlyph};
use crate::html::NodeId;
use crate::style::{ComputedStyle, FontStyle, FontWeight, RgbaColor};

use super::box_tree::InlineSpan;
use super::geometry::Rect;

/// 同一スタイル・同一フォントで連続する区間(1単語の一部、または1単語全体)。
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// この区間の描画に使う、[`FontCollection`]内でのフォントのインデックス。
    pub font_index: usize,
    pub font_size: f32,
    pub color: RgbaColor,
    pub bold: bool,
    pub italic: bool,
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

/// 1文字とその文字が属する[`InlineSpan`](=計算スタイル)への参照。
#[derive(Debug, Clone, Copy)]
struct StyledChar {
    ch: char,
    style_index: usize,
}

/// `spans`(テキストノード単位の区間列)を`available_width`に収まるよう行分割し、
/// `(origin_x, origin_y)`を起点に縦に積んだ行ボックス列を返す。単語の途中で
/// スタイル(`<b>`等)やフォント(CSSの`font-family`フォールバック)が切り替わる
/// 場合は、その単語を複数の[`TextRun`]に分けてシェイピングする。
pub fn layout_inline_content(
    spans: &[InlineSpan],
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    available_width: f32,
    origin_x: f32,
    origin_y: f32,
) -> Vec<LineBox> {
    if fonts.is_empty() || spans.is_empty() {
        return Vec::new();
    }

    let (chars, span_styles) = flatten_spans(spans, styles);
    let words = split_into_words(&chars);
    if words.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width = 0.0f32;
    let mut cursor_y = origin_y;

    for word in words {
        let mut word_runs = split_word_into_runs(word, &span_styles, fonts);
        let word_width: f32 = word_runs.iter().map(|r| r.width).sum();

        let space_width = current_runs
            .last()
            .map(|last| measure_space_width(fonts, last.font_index, last.font_size))
            .unwrap_or(0.0);

        if !current_runs.is_empty() && current_width + space_width + word_width > available_width {
            let line_height = line_height_for(&current_runs);
            lines.push(finish_line(
                std::mem::take(&mut current_runs),
                current_width,
                origin_x,
                cursor_y,
                line_height,
            ));
            cursor_y += line_height;
            current_width = 0.0;
        } else if !current_runs.is_empty() {
            current_width += space_width;
        }

        for run in &mut word_runs {
            run.x_offset = current_width;
            current_width += run.width;
        }
        current_runs.extend(word_runs);
    }

    if !current_runs.is_empty() {
        let line_height = line_height_for(&current_runs);
        lines.push(finish_line(
            current_runs,
            current_width,
            origin_x,
            cursor_y,
            line_height,
        ));
    }

    lines
}

/// `spans`を1文字単位に展開し、各文字が元のどの[`ComputedStyle`]に属するかの
/// インデックスを付与する。`span_styles`は文字と対になるスタイルの実体。
fn flatten_spans(
    spans: &[InlineSpan],
    styles: &HashMap<NodeId, ComputedStyle>,
) -> (Vec<StyledChar>, Vec<ComputedStyle>) {
    let mut chars = Vec::new();
    let mut span_styles = Vec::with_capacity(spans.len());

    for span in spans {
        let style = styles.get(&span.node).cloned().unwrap_or_default();
        let style_index = span_styles.len();
        span_styles.push(style);
        chars.extend(span.text.chars().map(|ch| StyledChar { ch, style_index }));
    }

    (chars, span_styles)
}

/// `char::is_whitespace`基準で`str::split_whitespace`相当に単語分割する
/// (連続する空白は畳み込み、先頭・末尾の空白は無視する)。
fn split_into_words(chars: &[StyledChar]) -> Vec<&[StyledChar]> {
    chars
        .split(|sc| sc.ch.is_whitespace())
        .filter(|word| !word.is_empty())
        .collect()
}

/// 単語を、(スタイル, フォント)が連続する区間ごとに[`TextRun`]へ分割する。
fn split_word_into_runs(
    word: &[StyledChar],
    span_styles: &[ComputedStyle],
    fonts: &FontCollection,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut current_text = String::new();

    for sc in word {
        let style = &span_styles[sc.style_index];
        let font_index = fonts
            .select_for_char(&style.font_family, sc.ch)
            .unwrap_or(0);

        match current {
            Some((style_index, fi)) if style_index == sc.style_index && fi == font_index => {
                current_text.push(sc.ch);
            }
            Some((style_index, fi)) => {
                runs.push(shape_run(
                    &current_text,
                    fi,
                    fonts,
                    &span_styles[style_index],
                ));
                current_text = sc.ch.to_string();
                current = Some((sc.style_index, font_index));
            }
            None => {
                current_text.push(sc.ch);
                current = Some((sc.style_index, font_index));
            }
        }
    }
    if let Some((style_index, fi)) = current {
        runs.push(shape_run(
            &current_text,
            fi,
            fonts,
            &span_styles[style_index],
        ));
    }

    runs
}

fn shape_run(
    text: &str,
    font_index: usize,
    fonts: &FontCollection,
    style: &ComputedStyle,
) -> TextRun {
    let font = fonts.get(font_index).expect("font_indexは常に有効な範囲");
    let font_size = style.font_size.0;
    let shaped = shape_text(font, text, font_size);
    TextRun {
        font_index,
        font_size,
        color: style.color,
        bold: style.font_weight == FontWeight::Bold,
        italic: style.font_style == FontStyle::Italic,
        text: text.to_string(),
        glyphs: shaped.glyphs,
        x_offset: 0.0,
        width: shaped.width,
    }
}

fn measure_space_width(fonts: &FontCollection, font_index: usize, font_size: f32) -> f32 {
    let Some(font) = fonts.get(font_index) else {
        return 0.0;
    };
    measure_text(font, " ", font_size)
}

/// 行内の各ランのフォントサイズのうち最大値を基準に行の高さを決める。
fn line_height_for(runs: &[TextRun]) -> f32 {
    runs.iter()
        .map(|r| r.font_size * 1.2)
        .fold(0.0f32, f32::max)
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
    use crate::html::{self, Dom};
    use crate::layout::box_tree::{build_box_tree, BoxContent, LayoutBox};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

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

    fn find_inline_spans(b: &LayoutBox) -> Option<&Vec<InlineSpan>> {
        match &b.content {
            BoxContent::Inline(spans) => Some(spans),
            BoxContent::Blocks(children) => children.iter().find_map(find_inline_spans),
        }
    }

    /// `<p>{inner_html}</p>`をパースし、最初のインラインボックスの
    /// スパン列と計算スタイルを返す(実際のDOM→ボックスツリー経由のテスト用)。
    fn spans_for(
        inner_html: &str,
        css: &str,
    ) -> (Dom, Vec<InlineSpan>, HashMap<NodeId, ComputedStyle>) {
        let html_src = format!("<p>{inner_html}</p>");
        let dom = html::parse(html_src.as_bytes());
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(css);
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let spans = find_inline_spans(&tree)
            .expect("expected inline content")
            .clone();
        (dom, spans, styles)
    }

    #[test]
    fn empty_or_whitespace_only_text_produces_no_lines() {
        let (_, spans, styles) = spans_for("", "");
        let fonts = dejavu_only();
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0).is_empty());

        let (_, spans, styles) = spans_for("   \n\t  ", "");
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn empty_font_collection_produces_no_lines() {
        let (_, spans, styles) = spans_for("hello", "");
        let fonts = FontCollection::new(vec![]);
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn text_that_fits_stays_on_a_single_line() {
        let (_, spans, styles) = spans_for("hello world", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 10.0, 20.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].rect.x, 10.0);
        assert_eq!(lines[0].rect.y, 20.0);
        assert!(lines[0].rect.width > 0.0);
        assert_eq!(
            lines[0].rect.height,
            ComputedStyle::default().font_size.0 * 1.2
        );
        // "hello"と"world"それぞれ1ランクずつ、同じフォントで連続。
        assert_eq!(lines[0].runs.len(), 2);
        assert!(lines[0].runs.iter().all(|r| r.font_index == 0));
    }

    #[test]
    fn wraps_to_a_new_line_when_available_width_is_too_narrow() {
        let fonts = dejavu_only();

        let (_, spans, styles) = spans_for("hello world foo bar", "");
        let one_line = layout_inline_content(&spans, &styles, &fonts, 1000.0, 0.0, 0.0);
        assert_eq!(one_line.len(), 1);

        let wrapped = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0);
        assert!(wrapped.len() > 1);

        let line_height = ComputedStyle::default().font_size.0 * 1.2;
        assert_eq!(wrapped[1].rect.y, wrapped[0].rect.y + line_height);
    }

    #[test]
    fn overlong_single_word_is_not_split_and_still_placed() {
        let (_, spans, styles) = spans_for("supercalifragilisticexpialidocious", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 10.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].rect.width > 10.0,
            "overflowing word should not be dropped or split"
        );
    }

    #[test]
    fn collapses_runs_of_whitespace_between_words() {
        let (_, spans, styles) = spans_for("a    b\n\tc", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        // 3単語、それぞれ1ランク。
        assert_eq!(lines[0].runs.len(), 3);
    }

    #[test]
    fn mixed_script_word_splits_into_separate_font_runs() {
        // 空白なしでLatinとCJKが混在する1トークン。
        let (_, spans, styles) = spans_for("café日本語", "");
        let fonts = dejavu_and_cjk();

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

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
        let (_, spans, styles) = spans_for("Invoice 請求書", "");
        let fonts = dejavu_and_cjk();

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2);
        assert_eq!(lines[0].runs[0].font_index, 0);
        assert_eq!(lines[0].runs[1].font_index, 1);
    }

    #[test]
    fn bold_span_in_the_middle_of_a_word_splits_into_separate_runs() {
        // "bo"は通常、"ld"は<b>(太字)というスタイル境界が単語の途中にある。
        let (_, spans, styles) = spans_for("bo<b>ld</b>", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2, "should split at the <b> boundary");
        assert!(!lines[0].runs[0].bold);
        assert!(lines[0].runs[1].bold);
        assert_eq!(lines[0].runs[0].text, "bo");
        assert_eq!(lines[0].runs[1].text, "ld");
    }

    #[test]
    fn inline_span_color_and_style_are_carried_onto_the_text_run() {
        let (_, spans, styles) = spans_for(
            r#"plain <em style="color: rgb(200, 0, 0);">urgent</em>"#,
            "",
        );
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        let plain_run = lines[0]
            .runs
            .iter()
            .find(|r| r.text == "plain")
            .expect("plain run not found");
        assert!(!plain_run.italic);
        assert_eq!(plain_run.color, ComputedStyle::default().color);

        let urgent_run = lines[0]
            .runs
            .iter()
            .find(|r| r.text == "urgent")
            .expect("urgent run not found");
        assert!(urgent_run.italic, "<em> should render in italic");
        assert_eq!(
            urgent_run.color,
            RgbaColor {
                red: 200,
                green: 0,
                blue: 0,
                alpha: 1.0
            }
        );
    }
}
