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
use crate::style::{
    ComputedStyle, FontStyle, FontWeight, LengthPercentage, LineHeight, RgbaColor, TextAlign,
    TextTransform, WhiteSpace,
};

use super::box_tree::InlineSpan;
use super::float_ctx::FloatContext;
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
    /// このランの計算済み行高さ(px)。`line-height: normal`は`font_size*1.2`の
    /// 近似、`<number>`はこのランの`font_size`で乗算済み([0020](
    /// ../../../docs/decisions/0020-typography-details-design.md)決定3)。
    pub line_height: f32,
    /// `letter-spacing`の解決済みpx。PDF描画層(`pdf::document::render_line`)が
    /// `Tc`(character spacing)としてそのまま使う(決定2、レイアウト側の幅計算
    /// にも反映済み)。
    pub letter_spacing: f32,
    /// `word-spacing`の解決済みpx。単語間gap計算専用(描画には使わない、
    /// 決定1: PDFの`Tw`は複合フォントに効かないためgap幅への加算だけで実現)。
    pub word_spacing: f32,
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
    /// `<br>`由来の強制改行文字かどうか([0037](
    /// ../../../docs/decisions/0037-forced-line-break-design.md)決定1)。
    /// `ch`は`'\n'`。`white-space: pre`の経路はこのフラグを見ずに`'\n'`だけで
    /// 行を分割するため、`<pre>`内の`<br>`も自然に改行になる。
    is_forced_break: bool,
}

/// 通常フロー(`white-space: normal`/`nowrap`)の行組みの入力単位
/// ([0037]決定2)。
enum InlineItem<'a> {
    Word(&'a [StyledChar]),
    /// `<br>`由来の強制改行。`style_index`は`<br>`要素自身のスタイル
    /// (空行の高さ算出に使う)。
    ForcedBreak {
        style_index: usize,
    },
}

/// `spans`(テキストノード単位の区間列)を`available_width`に収まるよう行分割し、
/// `(origin_x, origin_y)`を起点に縦に積んだ行ボックス列を返す。単語の途中で
/// スタイル(`<b>`等)やフォント(CSSの`font-family`フォールバック)が切り替わる
/// 場合は、その単語を複数の[`TextRun`]に分けてシェイピングする。
///
/// `float_ctx`が`Some`の場合、各行の開始時点でその行のY位置における
/// float占有帯を問い合わせ、`available_width`/`origin_x`を動的に狭める
/// (float周りのテキスト回り込み、[0019](
/// ../../../docs/decisions/0019-float-clear-position-relative-design.md)参照)。
/// `None`(floatが無い、またはテーブル列幅の事前測定など無関係な呼び出し)なら
/// 固定の`available_width`/`origin_x`のまま(既存動作)。
pub(crate) fn layout_inline_content(
    spans: &[InlineSpan],
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    available_width: f32,
    origin_x: f32,
    origin_y: f32,
    float_ctx: Option<&FloatContext>,
) -> Vec<LineBox> {
    if fonts.is_empty() || spans.is_empty() {
        return Vec::new();
    }

    let (chars, span_styles) = flatten_spans(spans, styles);
    // `text-align`/`text-indent`/`white-space`はIFC内の先頭spanの計算値で
    // 代表する(無名ボックスのbox_style欠陥を回避する設計、[0020]決定4)。
    let white_space = span_styles
        .first()
        .map(|s| s.white_space)
        .unwrap_or_default();
    let text_align = span_styles
        .first()
        .map(|s| s.text_align)
        .unwrap_or_default();
    // パーセンテージはこのIFCのcontaining width(`available_width`)基準で解決する
    // (`width`/`margin`と同じ「使用値は使う側で解決」パターン)。
    let text_indent = span_styles
        .first()
        .map(|s| resolve_length_percentage(s.text_indent, available_width))
        .unwrap_or(0.0);
    if white_space == WhiteSpace::Pre {
        return layout_pre_content(
            &chars,
            &span_styles,
            fonts,
            available_width,
            origin_x,
            origin_y,
            float_ctx,
        );
    }

    let items = split_into_items(&chars);
    if items.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width = 0.0f32;
    let mut cursor_y = origin_y;
    let mut line_left = origin_x;
    let mut line_available_width = available_width;
    // 現在組み立て中の行における、単語境界の位置(`current_runs`のインデックス)。
    // `text-align: justify`がここに追加スペースを配分する(行頭に来た単語は
    // 境界として記録しない、既存の行の左端そのものだから)。
    let mut word_boundaries: Vec<usize> = Vec::new();
    // 直前のアイテムが強制改行だった場合の、その`<br>`が要求する行高さ。
    // 末尾の`<br>`に対して空行を1つ足すため([0037]決定2-3)に使う。
    let mut trailing_break_height: Option<f32> = None;

    for item in items {
        let word = match item {
            InlineItem::Word(word) => {
                trailing_break_height = None;
                word
            }
            InlineItem::ForcedBreak { style_index } => {
                // 強制改行は行幅の残りに関係なく行を確定させる
                // (`white-space: nowrap`でも効く、[0037]決定2)。
                let break_height = span_styles
                    .get(style_index)
                    .map(resolve_line_height)
                    .unwrap_or(0.0);
                if current_runs.is_empty() {
                    // 行に何も無い状態での強制改行(連続する`<br>`や
                    // 段落先頭の`<br>`)は、高さだけを持つ空行になる(決定2-2)。
                    (line_left, line_available_width) =
                        line_band(float_ctx, cursor_y, break_height, origin_x, available_width);
                    lines.push(finish_line(
                        Vec::new(),
                        0.0,
                        line_left,
                        cursor_y,
                        break_height,
                    ));
                    cursor_y += break_height;
                } else {
                    let line_height = line_height_for(&current_runs);
                    lines.push(finish_line(
                        std::mem::take(&mut current_runs),
                        current_width,
                        line_left,
                        cursor_y,
                        line_height,
                    ));
                    // 強制改行で終わる行は最終行と同じ扱いで、`justify`の
                    // 伸縮対象にしない(決定2-1)。
                    apply_text_align(
                        lines.last_mut().expect("just pushed"),
                        text_align,
                        true,
                        line_available_width,
                        &word_boundaries,
                    );
                    word_boundaries.clear();
                    cursor_y += line_height;
                    current_width = 0.0;
                }
                // `<br clear="left|right|all">`(レガシー表示属性が`clear`
                // プロパティに変換されている、[0039](
                // ../../../docs/decisions/0039-presentational-attributes-design.md)
                // 決定5)。CSSで`br { clear: both }`と書いた場合も同じ経路。
                if let (Some(ctx), Some(clear)) =
                    (float_ctx, span_styles.get(style_index).map(|s| s.clear))
                {
                    cursor_y = ctx.clearance(clear, cursor_y);
                }
                trailing_break_height = Some(break_height);
                continue;
            }
        };
        let word_runs = split_word_into_runs(word, &span_styles, fonts);

        // 単語内であっても、CJK文字が絡む改行可能な境界ごとに「まとめて
        // 1行に収まるか判定する最小単位」(chunk)へグループ化する。空白による
        // 単語区切りは常に改行可能(次段の`is_first_chunk_of_word`で扱う)。
        for (chunk_index, chunk) in group_into_chunks(word_runs).into_iter().enumerate() {
            let chunk_width: f32 = chunk.iter().map(|r| r.width).sum();
            let is_first_chunk_of_word = chunk_index == 0;
            let starting_new_line = current_runs.is_empty();

            if starting_new_line {
                // 新しい行の先頭: floatに応じた帯を、このchunkのフォントサイズ
                // から近似した行高さ(`line_height_for`と同じ*1.2)で問い合わせる
                // (既知の簡略化: 行内でフォントサイズが極端に混在する場合は
                // 帯判定がわずかに不正確になり得るが、帳票用途では稀)。
                let hint = line_height_hint_for_chunk(&chunk);
                (line_left, line_available_width) =
                    line_band(float_ctx, cursor_y, hint, origin_x, available_width);
                // `text-indent`は最初の物理行のみに適用する(CSS2.1 §16.1)。
                if lines.is_empty() {
                    line_left += text_indent;
                    line_available_width -= text_indent;
                }
            }

            // 単語の先頭のchunkにのみ、直前のランとの間に単語間スペースを
            // 挟む。単語内のCJK境界で分かれた後続chunkは隙間0で直接続ける。
            let gap_width = if is_first_chunk_of_word {
                current_runs
                    .last()
                    .map(|last| {
                        measure_space_width(fonts, last.font_index, last.font_size)
                            + last.word_spacing
                    })
                    .unwrap_or(0.0)
            } else {
                0.0
            };

            if !starting_new_line
                && white_space != WhiteSpace::Nowrap
                && current_width + gap_width + chunk_width > line_available_width
            {
                let line_height = line_height_for(&current_runs);
                lines.push(finish_line(
                    std::mem::take(&mut current_runs),
                    current_width,
                    line_left,
                    cursor_y,
                    line_height,
                ));
                // 折返しで確定した行にtext-alignを適用する。最後の行ではない
                // ため`justify`も伸縮対象になる(CSS仕様: 最後の行は伸縮しない)。
                apply_text_align(
                    lines.last_mut().expect("just pushed"),
                    text_align,
                    false,
                    line_available_width,
                    &word_boundaries,
                );
                word_boundaries.clear();
                cursor_y += line_height;
                current_width = 0.0;

                let hint = line_height_hint_for_chunk(&chunk);
                (line_left, line_available_width) =
                    line_band(float_ctx, cursor_y, hint, origin_x, available_width);
            } else if !starting_new_line {
                if is_first_chunk_of_word {
                    word_boundaries.push(current_runs.len());
                }
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
            line_left,
            cursor_y,
            line_height,
        ));
        // 最後の行は`justify`で伸縮しない(CSS仕様)。
        apply_text_align(
            lines.last_mut().expect("just pushed"),
            text_align,
            true,
            line_available_width,
            &word_boundaries,
        );
    } else if let Some(break_height) = trailing_break_height {
        // 末尾の`<br>`は1行分の空行を残す(主要ブラウザと同じ挙動、
        // [0037]決定2-3)。
        let (left, _) = line_band(float_ctx, cursor_y, break_height, origin_x, available_width);
        lines.push(finish_line(Vec::new(), 0.0, left, cursor_y, break_height));
    }

    lines
}

/// 確定した行に`text-align`を適用する。`is_last_line`は`justify`が最後の行を
/// 伸縮しない(CSS仕様)ための判定。`word_boundaries`は行内の単語境界の位置
/// (`line.runs`のインデックス)で、`justify`がそこに追加スペースを配分する。
fn apply_text_align(
    line: &mut LineBox,
    text_align: TextAlign,
    is_last_line: bool,
    line_available_width: f32,
    word_boundaries: &[usize],
) {
    let leftover = line_available_width - line.rect.width;
    match text_align {
        TextAlign::Left => {}
        TextAlign::Right => shift_all_runs(line, leftover),
        TextAlign::Center => shift_all_runs(line, leftover / 2.0),
        TextAlign::Justify if !is_last_line && !word_boundaries.is_empty() && leftover > 0.0 => {
            let extra = leftover / word_boundaries.len() as f32;
            let mut shift = 0.0;
            for (i, run) in line.runs.iter_mut().enumerate() {
                if word_boundaries.contains(&i) {
                    shift += extra;
                }
                run.x_offset += shift;
            }
            line.rect.width = line_available_width;
        }
        TextAlign::Justify => {}
    }
}

fn shift_all_runs(line: &mut LineBox, shift: f32) {
    if shift <= 0.0 {
        return;
    }
    for run in &mut line.runs {
        run.x_offset += shift;
    }
}

/// `chunk`最初のランの計算済み`line_height`で行高さを近似する(帯を問い合わせる
/// 時点ではまだ行全体のランが確定していないため)。
fn line_height_hint_for_chunk(chunk: &[TextRun]) -> f32 {
    chunk.first().map(|r| r.line_height).unwrap_or(0.0)
}

/// `float_ctx`があれば`y`〜`y+height`の帯を問い合わせ、無ければ固定の
/// `(origin_x, available_width)`を返す。
fn line_band(
    float_ctx: Option<&FloatContext>,
    y: f32,
    height: f32,
    origin_x: f32,
    available_width: f32,
) -> (f32, f32) {
    match float_ctx {
        Some(ctx) => ctx.available_band(y, height, origin_x, origin_x + available_width),
        None => (origin_x, available_width),
    }
}

/// `spans`を1文字単位に展開し、各文字が元のどの[`ComputedStyle`]に属するかの
/// インデックスを付与する。`span_styles`は文字と対になるスタイルの実体。
/// `text-transform`はここで適用する(単語分割前の1パスで完結させる、[0020](
/// ../../../docs/decisions/0020-typography-details-design.md)参照)。
fn flatten_spans(
    spans: &[InlineSpan],
    styles: &HashMap<NodeId, ComputedStyle>,
) -> (Vec<StyledChar>, Vec<ComputedStyle>) {
    let mut chars = Vec::new();
    let mut span_styles = Vec::with_capacity(spans.len());
    // spanを跨いでも語頭判定を継続する(先頭は語頭扱い)。
    let mut prev_is_boundary = true;

    for span in spans {
        let mut style = styles.get(&span.node).cloned().unwrap_or_default();
        if span.is_first_letter {
            apply_first_letter_style(&mut style);
        }
        let style_index = span_styles.len();
        let transform = style.text_transform;
        span_styles.push(style);

        for ch in span.text.chars() {
            let is_word_start = prev_is_boundary;
            let transformed = apply_text_transform(ch, transform, is_word_start);
            chars.push(StyledChar {
                ch: transformed,
                style_index,
                is_forced_break: span.is_forced_break,
            });
            prev_is_boundary = ch.is_whitespace();
        }
    }

    (chars, span_styles)
}

/// `style.first_letter_style`(あれば)で対応するプロパティのみを上書きする
/// ([0024](../../../docs/decisions/0024-generated-content-design.md)決定4)。
fn apply_first_letter_style(style: &mut ComputedStyle) {
    let Some(first_letter) = style.first_letter_style.clone() else {
        return;
    };
    if let Some(v) = first_letter.font_size {
        style.font_size = v;
    }
    if let Some(v) = first_letter.font_family {
        style.font_family = v;
    }
    if let Some(v) = first_letter.font_weight {
        style.font_weight = v;
    }
    if let Some(v) = first_letter.font_style {
        style.font_style = v;
    }
    if let Some(v) = first_letter.color {
        style.color = v;
    }
    if let Some(v) = first_letter.text_decoration_line {
        style.text_decoration_line = v;
    }
    if let Some(v) = first_letter.text_transform {
        style.text_transform = v;
    }
}

/// `text-transform`を1文字に適用する。`uppercase`/`lowercase`は
/// `char::to_uppercase()`等の最初の1文字のみ採用する(独語ß等の複数文字展開は
/// 非対応、既知の簡略化)。`capitalize`は語頭の文字のみ変換する。
fn apply_text_transform(ch: char, transform: TextTransform, is_word_start: bool) -> char {
    match transform {
        TextTransform::None => ch,
        TextTransform::Uppercase => ch.to_uppercase().next().unwrap_or(ch),
        TextTransform::Lowercase => ch.to_lowercase().next().unwrap_or(ch),
        TextTransform::Capitalize if is_word_start && !ch.is_whitespace() => {
            ch.to_uppercase().next().unwrap_or(ch)
        }
        TextTransform::Capitalize => ch,
    }
}

/// `char::is_whitespace`基準で`str::split_whitespace`相当に単語分割しつつ、
/// `<br>`由来の強制改行を[`InlineItem::ForcedBreak`]として出現順に挟み込む
/// ([0037](../../../docs/decisions/0037-forced-line-break-design.md)決定2)。
/// 連続する空白は畳み込み、先頭・末尾の空白は無視する(強制改行は空白では
/// あるが畳み込まれず、常に1つのアイテムとして残る)。
fn split_into_items(chars: &[StyledChar]) -> Vec<InlineItem<'_>> {
    let mut items = Vec::new();
    let mut word_start = 0usize;

    for (i, sc) in chars.iter().enumerate() {
        if !sc.ch.is_whitespace() {
            continue;
        }
        if word_start < i {
            items.push(InlineItem::Word(&chars[word_start..i]));
        }
        if sc.is_forced_break {
            items.push(InlineItem::ForcedBreak {
                style_index: sc.style_index,
            });
        }
        word_start = i + 1;
    }
    if word_start < chars.len() {
        items.push(InlineItem::Word(&chars[word_start..]));
    }

    items
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

/// `LengthPercentage`を`basis`(containing width)を使ってpxへ解決する
/// (`block.rs::resolve_lp`と同じロジック、`text-indent`専用にここへ複製する)。
fn resolve_length_percentage(lp: LengthPercentage, basis: f32) -> f32 {
    match lp {
        LengthPercentage::Length(px) => px,
        LengthPercentage::Percentage(fraction) => fraction * basis,
    }
}

/// `line-height`の計算値からこの要素自身の`font_size`を使ってpx値を求める
/// ([0020](../../../docs/decisions/0020-typography-details-design.md)決定3:
/// `Number`/`Normal`は使用側=ここでその要素のfont-sizeを使って乗算する)。
fn resolve_line_height(style: &ComputedStyle) -> f32 {
    let font_size = style.font_size.0;
    match style.line_height {
        LineHeight::Normal => font_size * 1.2,
        LineHeight::Number(n) => n * font_size,
        LineHeight::Length(px) => px,
    }
}

/// `white-space: pre`用のレイアウト。改行文字(`\n`)で明示的に行を分割し、
/// 連続する空白はそのまま保持する(畳み込まない、`split_into_words`を経由しない)。
/// 折り返しは行わない(`nowrap`と同様、既存の`layout_inline_content`本体とは
/// 別経路にすることでNormal/Nowrap側のリグレッションリスクを避ける、[0020]参照)。
/// `split_word_into_runs`/`group_into_chunks`は変更せず再利用できるが、
/// `group_into_chunks`はCJK境界の改行可能判定用でpreでは折り返さないため
/// 使わず、`split_word_into_runs`の結果をそのまま1行に連結する。
fn layout_pre_content(
    chars: &[StyledChar],
    span_styles: &[ComputedStyle],
    fonts: &FontCollection,
    available_width: f32,
    origin_x: f32,
    origin_y: f32,
    float_ctx: Option<&FloatContext>,
) -> Vec<LineBox> {
    let text_indent = span_styles
        .first()
        .map(|s| resolve_length_percentage(s.text_indent, available_width))
        .unwrap_or(0.0);

    let mut lines = Vec::new();
    let mut cursor_y = origin_y;

    for segment in chars.split(|sc| sc.ch == '\n') {
        // 行高さの近似は、その行最初の文字のスタイル(無ければIFC先頭spanの
        // スタイル)を基準にする(既知の簡略化、[0020]決定5)。
        let hint = segment
            .first()
            .and_then(|sc| span_styles.get(sc.style_index))
            .or_else(|| span_styles.first())
            .map(resolve_line_height)
            .unwrap_or(0.0);
        let (mut line_left, _) = line_band(float_ctx, cursor_y, hint, origin_x, available_width);
        // `text-indent`は最初の物理行のみに適用する(CSS2.1 §16.1)。
        if lines.is_empty() {
            line_left += text_indent;
        }

        if segment.is_empty() {
            // 連続改行による空行。高さだけ消費するダミー行。
            lines.push(finish_line(Vec::new(), 0.0, line_left, cursor_y, hint));
            cursor_y += hint;
            continue;
        }

        let runs = split_word_into_runs(segment, span_styles, fonts);
        let mut current_width = 0.0;
        let mut placed_runs = Vec::with_capacity(runs.len());
        for mut run in runs {
            run.x_offset = current_width;
            current_width += run.width;
            placed_runs.push(run);
        }
        let line_height = line_height_for(&placed_runs);
        lines.push(finish_line(
            placed_runs,
            current_width,
            line_left,
            cursor_y,
            line_height,
        ));
        cursor_y += line_height;
    }

    lines
}

/// `list-style-type`のマーカーテキストのシェイピングにも使う
/// (`block.rs::layout_list_marker`、[0022](
/// ../../../docs/decisions/0022-list-style-design.md)決定4)ため`pub(super)`。
pub(super) fn shape_run(
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
    let line_height = resolve_line_height(style);
    // `letter-spacing`はグリフ数分だけ幅に加算する(行末にも均等加算する簡略化、
    // [0020]既知の簡略化2)。PDF描画層は`run.letter_spacing`を`Tc`として使う
    // ため、ここでの幅計算とレンダリング結果が一致する。
    let width = shaped.width + style.letter_spacing * shaped.glyphs.len() as f32;
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
        width,
        line_height,
        letter_spacing: style.letter_spacing,
        word_spacing: style.word_spacing,
    }
}

/// 任意の文字列を、折り返しなしの単一行として`(origin_x, origin_y)`起点で
/// シェイピングする。通常のDOMテキストノードを経由しない用途
/// (`@page`のmargin box、[0028](../../../docs/decisions/0028-paged-media-design.md)
/// 決定5)向け。文字ごとに`fonts.select_for_char`でフォントを選び直す
/// (`split_word_into_runs`と同じ考え方だが、折り返し判定が不要な分単純)。
pub fn shape_standalone_line(
    text: &str,
    style: &ComputedStyle,
    fonts: &FontCollection,
    origin_x: f32,
    origin_y: f32,
) -> LineBox {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut current_font: Option<usize> = None;
    let mut current_text = String::new();

    for ch in text.chars() {
        let font_index = fonts
            .select_for_char(&style.font_family, style.font_weight, style.font_style, ch)
            .unwrap_or(0);
        if current_font == Some(font_index) {
            current_text.push(ch);
        } else {
            if let Some(fi) = current_font {
                runs.push(shape_run(&current_text, fi, fonts, style));
            }
            current_text.clear();
            current_text.push(ch);
            current_font = Some(font_index);
        }
    }
    if let Some(fi) = current_font {
        runs.push(shape_run(&current_text, fi, fonts, style));
    }

    let mut x_cursor = 0.0;
    let mut max_height: f32 = 0.0;
    for run in &mut runs {
        run.x_offset = x_cursor;
        x_cursor += run.width;
        max_height = max_height.max(run.line_height);
    }

    LineBox {
        rect: Rect {
            x: origin_x,
            y: origin_y,
            width: x_cursor,
            height: max_height,
        },
        runs,
    }
}

fn measure_space_width(fonts: &FontCollection, font_index: usize, font_size: f32) -> f32 {
    let Some(font) = fonts.get(font_index) else {
        return 0.0;
    };
    measure_text(font, " ", font_size)
}

/// 行内の各ランの計算済み`line_height`のうち最大値を基準に行の高さを決める。
fn line_height_for(runs: &[TextRun]) -> f32 {
    runs.iter().map(|r| r.line_height).fold(0.0f32, f32::max)
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
                .caption
                .as_deref()
                .and_then(find_inline_spans)
                .or_else(|| {
                    table
                        .rows
                        .iter()
                        .flat_map(|row| &row.cells)
                        .find_map(|cell| find_inline_spans(&cell.content))
                }),
            BoxContent::Flex(flex) => flex.items.iter().find_map(find_inline_spans),
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
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0, None).is_empty());

        let (_, spans, styles) = spans_for("   \n\t  ", "");
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0, None).is_empty());
    }

    #[test]
    fn empty_font_collection_produces_no_lines() {
        let (_, spans, styles) = spans_for("hello", "");
        let fonts = FontCollection::new(vec![]);
        assert!(layout_inline_content(&spans, &styles, &fonts, 200.0, 0.0, 0.0, None).is_empty());
    }

    #[test]
    fn text_that_fits_stays_on_a_single_line() {
        let (_, spans, styles) = spans_for("hello world", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 10.0, 20.0, None);

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
    fn first_letter_style_overrides_are_applied_only_to_the_split_off_run() {
        let (_, spans, styles) = spans_for(
            "Hello world",
            "p::first-letter { font-size: 2em; color: rgb(200, 0, 0); font-weight: bold; }",
        );
        // real boldフェイスがないフォント集合を使い、synthetic boldフラグで
        // first-letterのfont-weightがランに反映されたことを検証する。
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        assert_eq!(lines.len(), 1);
        let runs = &lines[0].runs;
        assert!(runs.len() >= 2, "first-letter run + remainder run(s)");

        let base_font_size = ComputedStyle::default().font_size.0;
        assert_eq!(runs[0].text, "H");
        assert_eq!(runs[0].font_size, base_font_size * 2.0);
        assert_eq!(
            runs[0].color,
            RgbaColor {
                red: 200,
                green: 0,
                blue: 0,
                alpha: 1.0
            }
        );
        assert!(runs[0].bold);

        // 単語間の空白はラン間の隙間として表現され、`text`には含まれない。
        let remainder: String = runs[1..].iter().map(|r| r.text.as_str()).collect();
        assert_eq!(remainder, "elloworld");
        assert_eq!(runs[1].font_size, base_font_size);
        assert_eq!(runs[1].color, ComputedStyle::default().color);
        assert!(!runs[1].bold);
    }

    #[test]
    fn wraps_to_a_new_line_when_available_width_is_too_narrow() {
        let fonts = dejavu_only();

        let (_, spans, styles) = spans_for("hello world foo bar", "");
        let one_line = layout_inline_content(&spans, &styles, &fonts, 1000.0, 0.0, 0.0, None);
        assert_eq!(one_line.len(), 1);

        let wrapped = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0, None);
        assert!(wrapped.len() > 1);

        let line_height = ComputedStyle::default().font_size.0 * 1.2;
        assert_eq!(wrapped[1].rect.y, wrapped[0].rect.y + line_height);
    }

    #[test]
    fn float_narrows_the_band_for_lines_overlapping_it() {
        use crate::style::Float;

        let fonts = dejavu_only();
        let (_, spans, styles) = spans_for("hello world foo bar", "");

        // 左に400px幅・十分な高さのfloatを置き、全ての行がその右側
        // (x=400以降、幅100)に押し込まれることを確認する。
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 400.0, 1000.0);

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, Some(&ctx));
        assert!(!lines.is_empty());
        for line in &lines {
            assert_eq!(line.rect.x, 400.0);
            assert!(
                line.rect.width <= 100.0,
                "line width {} should not exceed the 100px band beside the float",
                line.rect.width
            );
        }
    }

    #[test]
    fn line_widens_back_after_passing_the_bottom_of_the_float() {
        use crate::style::Float;

        let fonts = dejavu_only();
        let (_, spans, styles) = spans_for("hello world foo bar baz", "");
        let line_height = ComputedStyle::default().font_size.0 * 1.2;

        // floatの高さは1行分だけ: 1行目はfloatの右に押し込まれ、2行目以降は
        // floatの下に出るため元の幅・左端に戻るはず。
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 400.0, line_height);

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, Some(&ctx));
        assert!(lines.len() >= 2, "expected wrapping to at least 2 lines");
        assert_eq!(lines[0].rect.x, 400.0);
        assert_eq!(
            lines[1].rect.x, 0.0,
            "second line should return to the full width once below the float"
        );
    }

    #[test]
    fn no_float_context_behaves_like_the_unconstrained_case() {
        let fonts = dejavu_only();
        let (_, spans, styles) = spans_for("hello world", "");

        let with_none = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let empty_ctx = FloatContext::new();
        let with_empty_ctx =
            layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, Some(&empty_ctx));

        assert_eq!(with_none, with_empty_ctx);
    }

    #[test]
    fn overlong_single_word_is_not_split_and_still_placed() {
        let (_, spans, styles) = spans_for("supercalifragilisticexpialidocious", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 10.0, 0.0, 0.0, None);

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
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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

        let narrow = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0, None);
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

        let wide = layout_inline_content(&spans, &styles, &fonts, 2000.0, 0.0, 0.0, None);
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
        let single_line = layout_inline_content(&spans, &styles, &fonts, 10000.0, 0.0, 0.0, None);
        let cafe_width = single_line[0].runs[0].width;

        let lines =
            layout_inline_content(&spans, &styles, &fonts, cafe_width + 1.0, 0.0, 0.0, None);
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
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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
    fn text_transform_uppercase_and_lowercase_apply_to_every_character() {
        let (_, spans, styles) = spans_for("Hello World", "p { text-transform: uppercase; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "HELLOWORLD");

        let (_, spans, styles) = spans_for("Hello World", "p { text-transform: lowercase; }");
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "helloworld");
    }

    #[test]
    fn text_transform_capitalize_affects_only_the_first_letter_of_each_word() {
        let (_, spans, styles) = spans_for("hello world", "p { text-transform: capitalize; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "HelloWorld");
    }

    #[test]
    fn text_transform_capitalize_treats_a_span_boundary_as_a_word_start() {
        // "hello <b>world</b>"のように単語の先頭がspan境界を跨いでいても、
        // capitalizeは正しく大文字化できるはず。
        let (_, spans, styles) =
            spans_for("hello <b>world</b>", "p { text-transform: capitalize; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "HelloWorld");
    }

    #[test]
    fn word_spacing_widens_the_gap_between_words() {
        let (_, spans, styles) = spans_for("hello world", "");
        let fonts = dejavu_only();
        let without = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        let (_, spans, styles) = spans_for("hello world", "p { word-spacing: 20px; }");
        let with = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        let gap_without =
            without[0].runs[1].x_offset - (without[0].runs[0].x_offset + without[0].runs[0].width);
        let gap_with =
            with[0].runs[1].x_offset - (with[0].runs[0].x_offset + with[0].runs[0].width);
        assert!(
            gap_with > gap_without,
            "word-spacing should widen the gap between words: without={gap_without}, with={gap_with}"
        );
    }

    #[test]
    fn letter_spacing_widens_run_width_by_glyph_count() {
        let (_, spans, styles) = spans_for("hello", "");
        let fonts = dejavu_only();
        let without = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        let (_, spans, styles) = spans_for("hello", "p { letter-spacing: 2px; }");
        let with = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        let glyph_count = with[0].runs[0].glyphs.len() as f32;
        assert_eq!(
            with[0].runs[0].width,
            without[0].runs[0].width + 2.0 * glyph_count
        );
        assert_eq!(with[0].runs[0].letter_spacing, 2.0);
    }

    #[test]
    fn white_space_nowrap_does_not_wrap_even_when_overflowing() {
        let (_, spans, styles) = spans_for("hello world foo bar", "p { white-space: nowrap; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0, None);

        assert_eq!(
            lines.len(),
            1,
            "nowrap should keep everything on a single line even when it overflows"
        );
        assert!(lines[0].rect.width > 60.0);
    }

    #[test]
    fn white_space_pre_preserves_explicit_newlines_and_does_not_wrap() {
        let (_, spans, styles) = spans_for(
            "hello&#10;world this is a long line",
            "p { white-space: pre; }",
        );
        let fonts = dejavu_only();
        // 幅を狭くしても、明示的な改行(\n)以外では折り返さないはず。
        let lines = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 10.0, None);

        assert_eq!(lines.len(), 2, "should split only at the explicit newline");
        assert_eq!(lines[0].runs.len(), 1);
        assert_eq!(lines[0].runs[0].text, "hello");
        assert!(
            lines[1].rect.width > 60.0,
            "the second physical line should not wrap despite overflowing"
        );
        let line_height = ComputedStyle::default().font_size.0 * 1.2;
        assert_eq!(lines[1].rect.y, lines[0].rect.y + line_height);
    }

    #[test]
    fn white_space_pre_preserves_runs_of_whitespace() {
        let (_, spans, styles) = spans_for("a   b", "p { white-space: pre; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        assert_eq!(lines.len(), 1);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "a   b", "runs of whitespace should not be collapsed");
    }

    #[test]
    fn white_space_pre_consecutive_newlines_produce_an_empty_line() {
        let (_, spans, styles) = spans_for("a&#10;&#10;b", "p { white-space: pre; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        assert_eq!(
            lines.len(),
            3,
            "two newlines should produce 3 physical lines"
        );
        assert!(lines[1].runs.is_empty(), "the middle line should be empty");
        assert!(
            lines[1].rect.height > 0.0,
            "an empty line still consumes height"
        );
    }

    #[test]
    fn text_align_left_is_the_default_and_does_not_shift_runs() {
        let (_, spans, styles) = spans_for("hi", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        assert_eq!(lines[0].runs[0].x_offset, 0.0);
    }

    #[test]
    fn text_align_right_pushes_the_line_to_the_right_edge() {
        let (_, spans, styles) = spans_for("hi", "p { text-align: right; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let content_width = lines[0].rect.width;
        assert_eq!(lines[0].runs[0].x_offset, 500.0 - content_width);
    }

    #[test]
    fn text_align_center_splits_the_leftover_space_evenly() {
        let (_, spans, styles) = spans_for("hi", "p { text-align: center; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        let content_width = lines[0].rect.width;
        assert_eq!(lines[0].runs[0].x_offset, (500.0 - content_width) / 2.0);
    }

    #[test]
    fn text_align_justify_spreads_extra_space_across_word_gaps_but_not_on_the_last_line() {
        let (_, spans, styles) = spans_for("hello world foo bar baz", "p { text-align: justify; }");
        let fonts = dejavu_only();
        // 幅を狭くして複数行に折り返させる。
        let lines = layout_inline_content(&spans, &styles, &fonts, 150.0, 0.0, 0.0, None);
        assert!(lines.len() >= 2, "expected wrapping to at least 2 lines");

        // 最後の行以外は、行幅ちょうど(available_width)まで引き伸ばされるはず。
        for line in &lines[..lines.len() - 1] {
            assert!(
                line.runs.len() >= 2,
                "a justified non-last line needs at least one word gap to stretch"
            );
            assert_eq!(
                line.rect.width, 150.0,
                "non-last justified lines should stretch to fill the available width"
            );
        }

        // 最後の行は伸縮しない(rect.widthは実際に使った幅のまま、150に届かない)。
        let last = lines.last().unwrap();
        assert!(
            last.rect.width < 150.0,
            "the last line should not be stretched by justify"
        );
    }

    #[test]
    fn text_align_justify_with_a_single_word_line_does_not_panic_or_shift() {
        // 単語境界が無い行(1単語だけ)はjustifyしても伸縮しない
        // (word_boundariesが空、既知の簡略化)。
        let (_, spans, styles) = spans_for(
            "supercalifragilisticexpialidocious",
            "p { text-align: justify; }",
        );
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 10.0, 0.0, 0.0, None);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].runs[0].x_offset, 0.0);
    }

    #[test]
    fn text_indent_px_shifts_only_the_first_line() {
        let (_, spans, styles) = spans_for("hello world foo bar", "p { text-indent: 30px; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 60.0, 0.0, 0.0, None);

        assert!(lines.len() >= 2, "expected wrapping to at least 2 lines");
        assert_eq!(lines[0].rect.x, 30.0);
        assert_eq!(lines[1].rect.x, 0.0, "second line should not be indented");
    }

    #[test]
    fn text_indent_percentage_resolves_against_available_width() {
        let (_, spans, styles) = spans_for("hi", "p { text-indent: 10%; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);
        assert_eq!(lines[0].rect.x, 50.0);
    }

    #[test]
    fn text_indent_applies_to_the_first_physical_line_of_pre_content() {
        let (_, spans, styles) = spans_for(
            "hello&#10;world",
            "p { white-space: pre; text-indent: 15px; }",
        );
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].rect.x, 15.0);
        assert_eq!(lines[1].rect.x, 0.0);
    }

    #[test]
    fn inline_span_color_and_style_are_carried_onto_the_text_run() {
        let (_, spans, styles) = spans_for(
            r#"plain <em style="color: rgb(200, 0, 0);">urgent</em>"#,
            "",
        );
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, None);

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

    /// 各行のテキストを連結して返す(強制改行のテスト用。空行は空文字列)。
    fn line_texts(lines: &[LineBox]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.runs.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn br_breaks_the_line_even_when_the_text_would_fit() {
        let (_, spans, styles) = spans_for("hello<br>world", "");
        let fonts = dejavu_only();
        // 十分に広い行幅でも改行される。
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);

        assert_eq!(line_texts(&lines), vec!["hello", "world"]);
        assert!(
            lines[1].rect.y > lines[0].rect.y,
            "the second line must be placed below the first"
        );
    }

    #[test]
    fn br_breaks_even_with_white_space_nowrap() {
        let (_, spans, styles) = spans_for("hello<br>world", "p { white-space: nowrap; }");
        let fonts = dejavu_only();
        // `nowrap`は「幅による折り返し」を止めるだけで、強制改行は効く。
        let lines = layout_inline_content(&spans, &styles, &fonts, 10.0, 0.0, 0.0, None);
        assert_eq!(line_texts(&lines), vec!["hello", "world"]);
    }

    #[test]
    fn consecutive_brs_produce_an_empty_line() {
        let (_, spans, styles) = spans_for("a<br><br>b", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);

        assert_eq!(line_texts(&lines), vec!["a", "", "b"]);
        assert!(
            lines[1].rect.height > 0.0,
            "the blank line must still take vertical space"
        );
        assert_eq!(lines[1].rect.y, lines[0].rect.y + lines[0].rect.height);
        assert_eq!(lines[2].rect.y, lines[1].rect.y + lines[1].rect.height);
    }

    #[test]
    fn a_trailing_br_leaves_one_empty_line() {
        // 主要ブラウザと同じ挙動([0037]決定2-3)。
        let (_, spans, styles) = spans_for("a<br>", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);

        assert_eq!(line_texts(&lines), vec!["a", ""]);
        assert!(lines[1].rect.height > 0.0);
    }

    #[test]
    fn a_leading_br_pushes_the_text_down_by_one_line() {
        let (_, spans, styles) = spans_for("<br>a", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);

        assert_eq!(line_texts(&lines), vec!["", "a"]);
        assert_eq!(lines[1].rect.y, lines[0].rect.height);
    }

    #[test]
    fn br_does_not_swallow_the_surrounding_words() {
        // 改行文字は単語区切りとしても働くため、前後の単語が連結されない。
        let (_, spans, styles) = spans_for("one two<br>three four", "");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);
        assert_eq!(line_texts(&lines), vec!["onetwo", "threefour"]);
    }

    #[test]
    fn br_inside_pre_also_breaks_the_line() {
        // `white-space: pre`は別経路(`layout_pre_content`)だが、`<br>`は
        // `'\n'`としてスパンに載るため改修なしで改行になる([0037]決定1)。
        let (_, spans, styles) = spans_for("a<br>b", "p { white-space: pre; }");
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);
        assert_eq!(line_texts(&lines), vec!["a", "b"]);
    }

    #[test]
    fn the_empty_line_of_a_br_uses_its_own_line_height() {
        let (_, spans, styles) = spans_for(
            "a<br><br>b",
            "p { font-size: 10px; } br { font-size: 40px; line-height: 2; }",
        );
        let fonts = dejavu_only();
        let lines = layout_inline_content(&spans, &styles, &fonts, 5000.0, 0.0, 0.0, None);

        assert_eq!(line_texts(&lines), vec!["a", "", "b"]);
        assert_eq!(
            lines[1].rect.height, 80.0,
            "the blank line takes the <br>'s own line-height (40px * 2)"
        );
    }

    #[test]
    fn br_clear_pushes_the_next_line_below_a_float() {
        // `<br clear="left">`はレガシー表示属性が`clear: left`に変換され
        // ([0039]決定5)、強制改行の直後の行をfloatの下端まで押し下げる。
        use crate::layout::float_ctx::FloatContext;
        use crate::style::Float;

        let (_, spans, styles) = spans_for("a<br clear=\"left\">b", "");
        let fonts = dejavu_only();
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 50.0, 100.0);

        let lines = layout_inline_content(&spans, &styles, &fonts, 500.0, 0.0, 0.0, Some(&ctx));
        assert_eq!(line_texts(&lines), vec!["a", "b"]);
        assert!(
            lines[1].rect.y >= 100.0,
            "the line after <br clear=left> must clear the float, got y={}",
            lines[1].rect.y
        );
    }
}
