//! Inline Formatting Context: 単純な貪欲法によるテキストの行分割と行ボックスの配置。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - `white-space: normal`相当の折り返し(連続する空白の畳み込み、単語単位の折り返し)
//!   のみ対応。長い単語1つで行幅を超える場合でも単語内では分割しない(ただし
//!   CJK文字が絡む境界は例外、後述)
//! - 単語間の空白の幅は、直前のテキストランのフォント・サイズを基準に測る
//!   (前後で大きくフォントサイズが異なる境界では厳密ではない)
//! - CJK文字(ひらがな・カタカナ・漢字・ハングル)が絡む境界は、空白が無くても
//!   改行可能とみなす(分かち書きをしない言語のため)。この判定のためだけに
//!   `split_word_into_runs`はスタイル/フォントが同じでもCJK境界では別ランに
//!   分ける(1文字ごとの個別シェイピングになる分の非効率とのトレードオフ)。
//!   UAX#14(Unicode Line Breaking Algorithm)の全面実装ではなく、
//!   「CJK文字が隣接する境界は改行可、それ以外はスタイル変更のみでは改行不可」
//!   という単純化した判定にとどめる

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
    pub underline: bool,
    pub line_through: bool,
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
        let word_runs = split_word_into_runs(word, &span_styles, fonts);

        // 単語内であっても、CJK文字が絡む改行可能な境界ごとに「まとめて
        // 1行に収まるか判定する最小単位」(chunk)へグループ化する。空白による
        // 単語区切りは常に改行可能(次段の`is_first_chunk_of_word`で扱う)。
        for (chunk_index, chunk) in group_into_chunks(word_runs).into_iter().enumerate() {
            let chunk_width: f32 = chunk.iter().map(|r| r.width).sum();
            let is_first_chunk_of_word = chunk_index == 0;

            // 単語の先頭のchunkにのみ、直前のランとの間に単語間スペースを
            // 挟む。単語内のCJK境界で分かれた後続chunkは隙間0で直接続ける。
            let gap_width = if is_first_chunk_of_word {
                current_runs
                    .last()
                    .map(|last| measure_space_width(fonts, last.font_index, last.font_size))
                    .unwrap_or(0.0)
            } else {
                0.0
            };

            if !current_runs.is_empty() && current_width + gap_width + chunk_width > available_width
            {
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
                current_width += gap_width;
            }

            for mut run in chunk {
                run.x_offset = current_width;
                current_width += run.width;
                current_runs.push(run);
            }
        }
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
/// CJK文字が絡む文字境界([`is_break_boundary`])では、スタイル/フォントが
/// 同じであっても別ランに分ける(改行可能な境界にするため。1文字ごとの
/// シェイピングになるが、CJK文字間の文脈依存シェイピングは通常無いため
/// 見た目には影響しない)。
fn split_word_into_runs(
    word: &[StyledChar],
    span_styles: &[ComputedStyle],
    fonts: &FontCollection,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    let mut current_text = String::new();
    let mut last_char: Option<char> = None;

    for sc in word {
        let style = &span_styles[sc.style_index];
        let font_index = fonts
            .select_for_char(
                &style.font_family,
                style.font_weight,
                style.font_style,
                sc.ch,
            )
            .unwrap_or(0);

        let continues_current = match (current, last_char) {
            (Some((style_index, fi)), Some(prev_ch)) => {
                style_index == sc.style_index
                    && fi == font_index
                    && !is_break_boundary(prev_ch, sc.ch)
            }
            _ => false,
        };

        if continues_current {
            current_text.push(sc.ch);
        } else {
            if let Some((style_index, fi)) = current {
                runs.push(shape_run(
                    &current_text,
                    fi,
                    fonts,
                    &span_styles[style_index],
                ));
            }
            current_text = sc.ch.to_string();
            current = Some((sc.style_index, font_index));
        }
        last_char = Some(sc.ch);
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

/// `runs`を、改行可能な境界(先頭、またはCJK文字が絡むrun境界
/// [`is_break_boundary`])ごとに分割不可能な塊(chunk)へグループ化する。
/// 各chunkの内部境界はすべて改行不可(スタイル/フォント変更のみ)なので、
/// 呼び出し側はchunk単位で「まとめて1行に収まるか」を判定できる。
fn group_into_chunks(runs: Vec<TextRun>) -> Vec<Vec<TextRun>> {
    let mut chunks: Vec<Vec<TextRun>> = Vec::new();
    for run in runs {
        let starts_new_chunk = match chunks.last().and_then(|chunk| chunk.last()) {
            None => true,
            Some(prev) => is_break_boundary(
                prev.text.chars().last().unwrap_or(' '),
                run.text.chars().next().unwrap_or(' '),
            ),
        };
        if starts_new_chunk {
            chunks.push(vec![run]);
        } else {
            chunks.last_mut().expect("just checked non-empty").push(run);
        }
    }
    chunks
}

/// `prev`と`next`の間で(空白が無くても)改行してよいかどうか。
/// どちらか一方がCJK文字([`is_cjk`])であれば改行可能とみなす簡略判定
/// (UAX#14の全面実装ではない)。
fn is_break_boundary(prev: char, next: char) -> bool {
    is_cjk(prev) || is_cjk(next)
}

/// ひらがな・カタカナ・漢字(CJK統合漢字・拡張A・互換漢字)・ハングルなど、
/// 分かち書きをしない(単語間に空白を置かない)スクリプトの文字かどうか。
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3000..=0x303F   // CJKの記号・句読点
        | 0x3040..=0x30FF // ひらがな・カタカナ
        | 0x31F0..=0x31FF // カタカナ拡張
        | 0x3400..=0x4DBF // CJK統合漢字拡張A
        | 0x4E00..=0x9FFF // CJK統合漢字
        | 0xAC00..=0xD7A3 // ハングル音節
        | 0xF900..=0xFAFF // CJK互換漢字
        | 0xFF00..=0xFFEF // 全角形・半角形
    )
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
    // 選択されたフォントが実際にBold/Italicであれば、疑似合成は不要
    // (`fonts::FontCollection::select_for_char`が本物のBold/Italic面を優先して
    // 選ぶため、`--font`/`@font-face`/システムフォントに実体があればここで
    // 疑似合成をスキップできる)。
    let needs_synthetic_bold = style.font_weight == FontWeight::Bold && !fonts.is_bold(font_index);
    let needs_synthetic_italic =
        style.font_style == FontStyle::Italic && !fonts.is_italic(font_index);
    TextRun {
        font_index,
        font_size,
        color: style.color,
        bold: needs_synthetic_bold,
        italic: needs_synthetic_italic,
        underline: style.text_decoration_line.underline,
        line_through: style.text_decoration_line.line_through,
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
    const DEJAVU_BOLD_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/DejaVuSans-Bold.ttf"
    );
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn dejavu_only() -> FontCollection {
        FontCollection::new(vec![Font::load(DEJAVU_PATH).unwrap()])
    }

    fn dejavu_regular_and_bold() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).unwrap(),
            Font::load(DEJAVU_BOLD_PATH).unwrap(),
        ])
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
            BoxContent::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .find_map(|cell| find_inline_spans(&cell.content)),
            BoxContent::Image(_) => None,
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
        // 空白なしでLatinとCJKが混在する1トークン。CJK文字(日本語)は
        // 改行可能境界のため、スタイル/フォントが同じでも1文字ずつ別ランに
        // 分かれる("café" + "日" + "本" + "語" = 4ラン)。
        let (_, spans, styles) = spans_for("café日本語", "");
        let fonts = dejavu_and_cjk();

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 4, "café / 日 / 本 / 語 の4ラン");
        assert_eq!(
            lines[0].runs[0].font_index, 0,
            "café should use DejaVu Sans"
        );
        assert_eq!(lines[0].runs[0].text, "café");
        for (run, expected_char) in lines[0].runs[1..].iter().zip(['日', '本', '語']) {
            assert_eq!(
                run.font_index, 1,
                "{expected_char} should use the CJK fallback font"
            );
            assert_eq!(run.text, expected_char.to_string());
        }
        // 各ランは隙間なく(単語内なので空白は挟まず)左から右へ連続する。
        let mut prev_end = lines[0].runs[0].x_offset + lines[0].runs[0].width;
        for run in &lines[0].runs[1..] {
            assert_eq!(run.x_offset, prev_end);
            prev_end = run.x_offset + run.width;
        }
    }

    #[test]
    fn separate_cjk_and_latin_words_can_land_on_the_same_line() {
        let (_, spans, styles) = spans_for("Invoice 請求書", "");
        let fonts = dejavu_and_cjk();

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        // "Invoice"は1ラン、"請求書"はCJKなので1文字ずつ3ランに分かれる。
        assert_eq!(lines[0].runs.len(), 4);
        assert_eq!(lines[0].runs[0].font_index, 0);
        assert_eq!(lines[0].runs[0].text, "Invoice");
        for run in &lines[0].runs[1..] {
            assert_eq!(run.font_index, 1);
        }
    }

    #[test]
    fn long_cjk_sequence_wraps_between_characters_without_whitespace() {
        // 空白の無い長いCJK文字列でも、行幅に収まらなければ文字間で改行できる
        // (分かち書きをしない言語のため)。
        let (_, spans, styles) = spans_for("日本語のテスト文章です", "");
        let fonts = dejavu_and_cjk();

        let narrow = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0);
        assert!(
            narrow.len() > 1,
            "a narrow line width should force wrapping within the CJK sequence"
        );
        for line in &narrow {
            assert!(
                !line.runs.is_empty(),
                "every wrapped line should contain at least one run"
            );
        }

        let wide = layout_inline_content(&spans, &styles, &fonts, 2000.0, 0.0, 0.0);
        assert_eq!(
            wide.len(),
            1,
            "a wide enough line should keep the whole sequence on one line"
        );
    }

    #[test]
    fn cafe_nihongo_wraps_between_the_script_boundary_when_narrow() {
        // タスクで名指しされていた具体例: "café日本語"のようにスペースが無い
        // まま行幅を超える場合、Latin/CJKの境界(または日本語文字の間)で
        // 改行できるはず(以前は1つの分割不能な単語として扱われ、行幅を
        // 超えてもはみ出したまま単一行に配置されていた)。
        let (_, spans, styles) = spans_for("café日本語", "");
        let fonts = dejavu_and_cjk();

        // "café"の幅ぎりぎりの行幅にすると、続く日本語部分は収まらないはず。
        let single_line = layout_inline_content(&spans, &styles, &fonts, 10000.0, 0.0, 0.0);
        let cafe_width = single_line[0].runs[0].width;

        let lines = layout_inline_content(&spans, &styles, &fonts, cafe_width + 1.0, 0.0, 0.0);
        assert!(
            lines.len() > 1,
            "should wrap at the café/日 boundary instead of overflowing as one unbreakable word"
        );
        assert_eq!(lines[0].runs.len(), 1);
        assert_eq!(lines[0].runs[0].text, "café");
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
    fn bold_span_uses_the_real_bold_face_and_skips_synthetic_bold_when_available() {
        // "bo"は通常、"ld"は<b>(太字)。フォントコレクションにDejaVu SansのBold版も
        // 含まれている場合、疑似太字ではなく本物のBold面が選ばれるはず
        // (family名を明示しないと既定の"sans-serif"はどちらのフォント名にも
        // 一致せず、weight/styleを問わない先頭フォントへのフォールバックに
        // 落ちてしまい本来テストしたい分岐を通らないため、明示的に指定する)。
        let (_, spans, styles) = spans_for("bo<b>ld</b>", "p { font-family: 'DejaVu Sans'; }");
        let fonts = dejavu_regular_and_bold();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2);
        assert_eq!(
            lines[0].runs[0].font_index, 0,
            "\"bo\" (normal weight) should use the regular face"
        );
        assert!(!lines[0].runs[0].bold);
        assert_eq!(
            lines[0].runs[1].font_index, 1,
            "\"ld\" (bold) should use the real bold face, not the regular one"
        );
        assert!(
            !lines[0].runs[1].bold,
            "no synthetic bold should be applied when a real bold face was selected"
        );
    }

    #[test]
    fn bold_span_prefers_the_real_bold_face_even_without_a_matching_font_family() {
        // font-familyを一切指定しない(既定値"sans-serif")場合でも、familyの
        // 一致を問わないグローバルフォールバック側でweight/style一致を優先し、
        // 本物のBold面を選べるはず(family一致だけを見ていた旧実装だと、
        // "sans-serif"はどのフォント名にも一致せずグリフ網羅性のみによる
        // フォールバックに落ちてしまい、太字要求を無視して先頭のRegular面が
        // 選ばれてしまっていた)。
        let (_, spans, styles) = spans_for("bo<b>ld</b>", "");
        let fonts = dejavu_regular_and_bold();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs.len(), 2);
        assert_eq!(lines[0].runs[0].font_index, 0);
        assert_eq!(
            lines[0].runs[1].font_index, 1,
            "bold text should still find the real bold face via the family-agnostic fallback"
        );
        assert!(!lines[0].runs[1].bold);
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
