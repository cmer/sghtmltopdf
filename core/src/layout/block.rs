//! Block Formatting Context: containing blockに基づく幅計算と、
//! ブロック要素の縦積み配置(CSS2.1 §10.3.3, §9.4.1の簡略版)。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - マージン相殺(margin collapsing)は隣接兄弟間のみ対応する(CSS2.1 §8.3.1)。
//!   親子間の相殺(親の上/下マージンと最初/最後の子のマージンの相殺)、および
//!   高さ0・border/paddingなしの空ブロックを上下マージンが素通りする相殺は
//!   未対応
//! - `direction: rtl`は未対応(常にltr前提。over-constrained時に再計算する辺は
//!   margin-right固定)
//! - 高さのパーセンテージ指定はcontaining blockの高さが不定なため`auto`として扱う
//! - インラインコンテンツの行分割・実際の行数に応じた高さはT6([`super::inline`])が担う
use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::{
    BorderStyle, BreakBetween, BreakInside, ComputedStyle, Display, Length, LengthPercentage,
    LengthPercentageOrAuto,
};

use super::box_tree::{BoxContent, LayoutBox};
use super::geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
use super::inline::{layout_inline_content, LineBox};
use super::table::layout_table;

#[derive(Debug, Clone)]
pub struct LaidOutBox {
    pub node: Option<NodeId>,
    pub layout: Layout,
    /// このボックスの`break-before`/`break-after`/`break-inside`/`orphans`/`widows`の
    /// 計算値(ページ分割の判断にのみ使う。無名ボックスは`ComputedStyle`の
    /// 初期値=`auto`/`auto`/`auto`/2/2)。
    pub fragmentation: FragmentationHints,
    /// このボックスが実際に描画される背景色・枠線を持つか。
    ///
    /// `paginate.rs`が、ページをまたいで分割されるコンテナの装飾フラグメント
    /// (背景・枠線の再現、モジュールdoc参照)を生成する必要があるかどうかの
    /// 判断に使う。`border-radius`の有無はここでは無関係(角丸があっても
    /// 背景色・枠線が両方なければ何も描画されないため、`pdf::document`側の
    /// 描画ロジックとは独立に判定してよい)。装飾フラグメント自体・行の
    /// 合成ラッパーなど無名ボックスは常に`false`(それ自体が再帰的に装飾
    /// フラグメントを持つことはない)。
    pub has_visible_decoration: bool,
    pub content: LaidOutContent,
}

/// [`LaidOutBox`]が持つCSS Fragmentation関連の計算値。ページ分割(`paginate.rs`)が
/// どこで分割するかを決める際に参照する(レイアウト自体には影響しない)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FragmentationHints {
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    pub break_inside: BreakInside,
    pub orphans: u32,
    pub widows: u32,
}

impl From<&ComputedStyle> for FragmentationHints {
    fn from(style: &ComputedStyle) -> Self {
        Self {
            break_before: style.break_before,
            break_after: style.break_after,
            break_inside: style.break_inside,
            orphans: style.orphans,
            widows: style.widows,
        }
    }
}

impl Default for FragmentationHints {
    fn default() -> Self {
        Self::from(&ComputedStyle::default())
    }
}

#[derive(Debug, Clone)]
pub enum LaidOutContent {
    Blocks(Vec<LaidOutBox>),
    Inline(Vec<LineBox>),
    Table(Vec<LaidOutTableRow>),
}

/// レイアウト済みのテーブル行1行分。
#[derive(Debug, Clone)]
pub struct LaidOutTableRow {
    pub node: NodeId,
    pub cells: Vec<LaidOutBox>,
}

/// ページ幅を初期containing blockとして、ボックスツリー全体をレイアウトする。
pub fn layout_document(
    root: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    page_width: f32,
) -> LaidOutBox {
    layout_document_from(root, styles, fonts, page_width, 0.0, 0.0)
}

/// [`layout_document`]のバリアント: 原点`(0.0, 0.0)`からではなく、
/// `(start_x, start_y)`からレイアウトを開始する。
///
/// マイルストーン3のストリーミング処理で、`<body>`直下のトップレベル要素を
/// 1つずつレイアウトする際、前の要素までの累積高さを`start_y`として渡す
/// ことで、複数回の呼び出しにまたがって「上から下に流れる」通常のブロック
/// レイアウトを継続する。`start_x`/`containing_width`には、`<body>`自身の
/// `margin`/`border`/`padding`を反映した値を渡すことを想定する(`<body>`
/// 自体は個々のトップレベル要素とは別に扱われ、その内側がこの関数の
/// containing blockになるため)。
pub fn layout_document_from(
    root: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    start_x: f32,
    start_y: f32,
) -> LaidOutBox {
    layout_box(root, styles, fonts, containing_width, start_x, start_y)
}

fn layout_box(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    x: f32,
    y: f32,
) -> LaidOutBox {
    layout_box_impl(b, styles, fonts, containing_width, None, x, y)
}

/// テーブルセルなど、通常の`width`解決(auto/margin計算)を経ずに
/// content-boxの幅を直接指定してレイアウトしたい場合に使う
/// ([`super::table`]専用)。
pub(super) fn layout_box_with_forced_width(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    forced_content_width: f32,
    x: f32,
    y: f32,
) -> LaidOutBox {
    layout_box_impl(
        b,
        styles,
        fonts,
        containing_width,
        Some(forced_content_width),
        x,
        y,
    )
}

fn layout_box_impl(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    forced_content_width: Option<f32>,
    x: f32,
    y: f32,
) -> LaidOutBox {
    let style = box_style(b, styles);

    let padding = resolve_padding(&style, containing_width);
    let border = resolve_border(&style);
    let (content_width, margin_left, margin_right) = match forced_content_width {
        Some(w) => (
            w,
            resolve_lpa_or_zero(style.margin_left, containing_width),
            resolve_lpa_or_zero(style.margin_right, containing_width),
        ),
        None => resolve_width_and_horizontal_margins(
            &style,
            containing_width,
            padding.left + padding.right,
            border.left + border.right,
        ),
    };
    let margin = EdgeSizes {
        top: resolve_lpa_or_zero(style.margin_top, containing_width),
        right: margin_right,
        bottom: resolve_lpa_or_zero(style.margin_bottom, containing_width),
        left: margin_left,
    };

    let content_x = x + margin.left + border.left + padding.left;
    let content_y = y + margin.top + border.top + padding.top;

    let (content, content_height) = match &b.content {
        BoxContent::Blocks(children) => {
            let mut cursor_y = content_y;
            let mut laid_children: Vec<LaidOutBox> = Vec::with_capacity(children.len());
            for child in children {
                let child_margin_top =
                    resolve_lpa_or_zero(box_style(child, styles).margin_top, content_width);

                // 隣接兄弟間のマージン相殺(CSS2.1 §8.3.1)。前の兄弟のmargin-bottomと
                // この子のmargin-topを、単純な加算ではなく「正の最大値+負の最小値」
                // で相殺した1つの間隔に置き換える。
                if let Some(prev) = laid_children.last() {
                    let prev_margin_bottom = prev.layout.margin.bottom;
                    let collapsed = collapse_adjacent_margins(prev_margin_bottom, child_margin_top);
                    cursor_y -= prev_margin_bottom + child_margin_top - collapsed;
                }

                let child_laid =
                    layout_box(child, styles, fonts, content_width, content_x, cursor_y);
                cursor_y += child_laid.layout.margin_box_height();
                laid_children.push(child_laid);
            }
            let auto_height = cursor_y - content_y;
            let height = resolve_height(&style).unwrap_or(auto_height);
            (LaidOutContent::Blocks(laid_children), height)
        }
        BoxContent::Inline(spans) => {
            let lines =
                layout_inline_content(spans, styles, fonts, content_width, content_x, content_y);
            let lines_height: f32 = lines.iter().map(|line| line.rect.height).sum();
            let height = resolve_height(&style).unwrap_or(lines_height);
            (LaidOutContent::Inline(lines), height)
        }
        BoxContent::Table(table) => {
            let (rows, table_height) =
                layout_table(table, styles, fonts, content_width, content_x, content_y);
            let height = resolve_height(&style).unwrap_or(table_height);
            (LaidOutContent::Table(rows), height)
        }
    };

    LaidOutBox {
        node: b.node,
        layout: Layout {
            content: Rect {
                x: content_x,
                y: content_y,
                width: content_width,
                height: content_height,
            },
            padding,
            border,
            margin,
            fragment: FragmentPosition::Whole,
        },
        fragmentation: FragmentationHints::from(&style),
        has_visible_decoration: has_visible_decoration(&style, &border),
        content,
    }
}

/// `style`/`border`(計算済みの太さ)の組み合わせが、実際に何か描画するか。
/// 背景色があるか、4辺のいずれかで太さが正かつ`border-style`が`none`でない
/// 場合に`true`(`pdf::document::render_box_decoration`が実際に描画する
/// 条件と同じ)。マイルストーン3の`Engine`が、`<body>`自身に装飾がないか
/// 判定する際にも使うため`pub(crate)`にしている。
pub(crate) fn has_visible_decoration(style: &ComputedStyle, border: &EdgeSizes) -> bool {
    if style.background_color.alpha > 0.0 {
        return true;
    }
    [
        (border.top, style.border_top_style),
        (border.right, style.border_right_style),
        (border.bottom, style.border_bottom_style),
        (border.left, style.border_left_style),
    ]
    .into_iter()
    .any(|(width, border_style)| width > 0.0 && border_style != BorderStyle::None)
}

pub(super) fn box_style(b: &LayoutBox, styles: &HashMap<NodeId, ComputedStyle>) -> ComputedStyle {
    match b.node {
        Some(node) => styles[&node].clone(),
        // 無名ボックス(CSS2.1 9.2.1.1)。マージン/パディング/枠線を持たないblock。
        None => ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        },
    }
}

fn resolve_lp(lp: LengthPercentage, basis: f32) -> f32 {
    match lp {
        LengthPercentage::Length(px) => px,
        LengthPercentage::Percentage(fraction) => fraction * basis,
    }
}

pub(crate) fn resolve_lpa_or_zero(lpa: LengthPercentageOrAuto, basis: f32) -> f32 {
    match lpa {
        LengthPercentageOrAuto::Auto => 0.0,
        LengthPercentageOrAuto::LengthPercentage(lp) => resolve_lp(lp, basis),
    }
}

pub(crate) fn resolve_padding(style: &ComputedStyle, basis: f32) -> EdgeSizes {
    EdgeSizes {
        top: resolve_lp(style.padding_top, basis),
        right: resolve_lp(style.padding_right, basis),
        bottom: resolve_lp(style.padding_bottom, basis),
        left: resolve_lp(style.padding_left, basis),
    }
}

/// `border-style: none`の辺は、`border-width`の指定に関わらず使用値が`0`になる
/// (CSS2.1 8.5.3)。レイアウト(幅計算)にもこの丸めが反映される必要がある。
pub(crate) fn resolve_border(style: &ComputedStyle) -> EdgeSizes {
    let width_or_zero = |width: Length, border_style: BorderStyle| {
        if border_style == BorderStyle::None {
            0.0
        } else {
            width.0
        }
    };
    EdgeSizes {
        top: width_or_zero(style.border_top_width, style.border_top_style),
        right: width_or_zero(style.border_right_width, style.border_right_style),
        bottom: width_or_zero(style.border_bottom_width, style.border_bottom_style),
        left: width_or_zero(style.border_left_width, style.border_left_style),
    }
}

/// `height`が明示指定されていれば返す。`auto`および(containing blockの高さが
/// 不定なため)パーセンテージ指定は`None`とし、呼び出し側でコンテンツ高さを使う。
fn resolve_height(style: &ComputedStyle) -> Option<f32> {
    match style.height {
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => Some(px),
        LengthPercentageOrAuto::Auto | LengthPercentageOrAuto::LengthPercentage(_) => None,
    }
}

/// 2つの隣接するマージンを相殺(collapse)した結果の間隔を求める(CSS2.1 §8.3.1)。
/// 両方が非負なら大きい方、両方が負なら小さい方(絶対値が大きい方)、
/// 正負混在なら両者の単純な和(=正の最大値と負の最小値の和)になる。
fn collapse_adjacent_margins(a: f32, b: f32) -> f32 {
    let positive = a.max(0.0).max(b.max(0.0));
    let negative = a.min(0.0).min(b.min(0.0));
    positive + negative
}

/// CSS2.1 §10.3.3(block-level, non-replaced要素)の簡略版。
/// `margin-left + border-left + padding-left + width + padding-right + border-right + margin-right
/// = containing blockの幅`という制約から、`auto`な項目を埋める。
pub(crate) fn resolve_width_and_horizontal_margins(
    style: &ComputedStyle,
    containing_width: f32,
    padding_lr: f32,
    border_lr: f32,
) -> (f32, f32, f32) {
    let margin_left_is_auto = matches!(style.margin_left, LengthPercentageOrAuto::Auto);
    let margin_right_is_auto = matches!(style.margin_right, LengthPercentageOrAuto::Auto);

    if matches!(style.width, LengthPercentageOrAuto::Auto) {
        let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
        let margin_right = resolve_lpa_or_zero(style.margin_right, containing_width);
        let width =
            (containing_width - margin_left - border_lr - padding_lr - margin_right).max(0.0);
        return (width, margin_left, margin_right);
    }

    let width = resolve_lpa_or_zero(style.width, containing_width);
    let remaining = (containing_width - border_lr - padding_lr - width).max(0.0);

    match (margin_left_is_auto, margin_right_is_auto) {
        (true, true) => {
            let half = remaining / 2.0;
            (width, half, half)
        }
        (true, false) => {
            let margin_right = resolve_lpa_or_zero(style.margin_right, containing_width);
            (width, (remaining - margin_right).max(0.0), margin_right)
        }
        (false, true) => {
            let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
            (width, margin_left, (remaining - margin_left).max(0.0))
        }
        (false, false) => {
            // over-constrained(CSS2.1 §10.3.3): width/margin-left/margin-rightが
            // 全て明示指定されている場合、指定されたmargin-rightの値は無視し、
            // 等式(margin-left + border/padding + width + margin-right =
            // containing width)がちょうど成り立つよう使用値を再計算する
            // (負の値になることもある。`direction: rtl`時はmargin-left側を
            // 再計算すべきだが、rtl自体が未対応のため常にltr前提)。
            let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
            let margin_right = containing_width - border_lr - padding_lr - width - margin_left;
            (width, margin_left, margin_right)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::box_tree::build_box_tree;
    use super::*;
    use crate::fonts::Font;
    use crate::html::{self, Dom, NodeData};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(TEST_FONT_PATH).expect("should load bundled test font")
        ])
    }

    fn find_all(dom: &Dom, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                out.push(id);
            }
        }
        for child in dom.children(id) {
            find_all(dom, child, tag, out);
        }
    }

    fn find_box(b: &LayoutBox, target: NodeId) -> Option<&LayoutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        if let BoxContent::Blocks(children) = &b.content {
            for child in children {
                if let Some(found) = find_box(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        if let LaidOutContent::Blocks(children) = &b.content {
            for child in children {
                if let Some(found) = find_laid_out(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn display_none_excludes_element_and_subtree() {
        let dom = html::parse(
            br#"<div><p class="hidden">hidden</p><p class="visible">visible</p></div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".hidden { display: none; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let (hidden_p, visible_p) = (ps[0], ps[1]);

        assert!(find_box(&tree, hidden_p).is_none());
        assert!(find_box(&tree, visible_p).is_some());
    }

    #[test]
    fn mixed_block_and_inline_children_get_anonymous_block_wrapping() {
        let dom = html::parse(br#"<div class="outer">before <p>P</p> after</div>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);

        let div_box = find_box(&tree, divs[0]).expect("div box not found");
        let BoxContent::Blocks(children) = &div_box.content else {
            panic!("expected block container")
        };
        assert_eq!(children.len(), 3, "before-text / <p> / after-text");
        let joined_text = |content: &BoxContent| match content {
            BoxContent::Inline(spans) => spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            BoxContent::Blocks(_) | BoxContent::Table(_) => panic!("expected inline content"),
        };
        assert_eq!(joined_text(&children[0].content).trim(), "before");
        assert_eq!(children[1].node, Some(ps[0]));
        assert_eq!(joined_text(&children[2].content).trim(), "after");
    }

    #[test]
    fn auto_width_fills_containing_block_minus_margins() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".box { margin: 10px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        // html: margin/padding/borderなし → content_width=800
        // body: UAデフォルトのmargin:8px → content_width=784
        // div: margin:10px → content_width=764
        assert_eq!(div_box.layout.margin.left, 10.0);
        assert_eq!(div_box.layout.content.width, 764.0);
        assert_eq!(div_box.layout.content.x, 18.0);
    }

    #[test]
    fn auto_margins_center_element_with_explicit_width() {
        let dom = html::parse(br#"<div class="centered"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".centered { width: 400px; margin: 0 auto; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 400.0);
        assert_eq!(div_box.layout.margin.left, div_box.layout.margin.right);
        assert_eq!(div_box.layout.margin.left, 192.0);
    }

    #[test]
    fn over_constrained_box_recalculates_margin_right_to_fit_the_containing_block() {
        // width/margin-left/margin-rightが全て明示指定され、かつ合計が
        // containing widthと一致しない(over-constrained)場合、CSS2.1 §10.3.3
        // に従い指定されたmargin-rightは無視され、等式が成り立つよう再計算される。
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        // containing width = 784(html:800, body margin:8pxずつ)。
        // width:300 + margin-left:50 + 指定margin-right:50 = 400 だが、
        // 784になるようmargin-rightは434に再計算されるはず。
        let author = parse_stylesheet(".box { width: 300px; margin: 0 50px 0 50px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 300.0);
        assert_eq!(div_box.layout.margin.left, 50.0);
        assert_eq!(
            div_box.layout.margin.right, 434.0,
            "over-constrained margin-right should be recalculated, not the specified 50px"
        );
    }

    #[test]
    fn over_constrained_recalculation_can_produce_a_negative_margin_right() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        // containing width = 784。width自体がそれを埋め尽くすので、margin-leftの
        // 分だけ超過し、再計算後のmargin-rightは指定値(99px)と符号すら異なる
        // 負の値になるはず。
        let author = parse_stylesheet(".box { width: 784px; margin: 0 99px 0 30px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.margin.right, -30.0);
    }

    #[test]
    fn block_siblings_stack_vertically_by_content_height() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet(".a { height: 50px; margin: 0; } .b { height: 30px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        assert_eq!(
            b.layout.content.y,
            a.layout.content.y + a.layout.content.height
        );
    }

    #[test]
    fn equal_adjacent_margins_collapse_to_a_single_gap_instead_of_summing() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        // 両方とも上下16pxのマージン。相殺されていれば、border-box間の隙間は
        // 32px(単純な加算)ではなく16pxになるはず。
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 16px 0; } .b { height: 20px; margin: 16px 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 16.0,
            "equal adjacent margins should collapse to their shared value"
        );
    }

    #[test]
    fn unequal_adjacent_margins_collapse_to_the_larger_one() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 0 0 10px 0; } .b { height: 20px; margin: 24px 0 0 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 24.0,
            "collapsed gap should be the larger of the two margins"
        );
    }

    #[test]
    fn a_negative_margin_reduces_the_collapsed_gap() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 0 0 10px 0; } .b { height: 20px; margin: -4px 0 0 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 6.0,
            "positive + negative margins should sum (10 + (-4) = 6)"
        );
    }

    #[test]
    fn parent_and_first_child_margins_are_not_collapsed() {
        // 親子間のマージン相殺は本実装のスコープ外(隣接兄弟間のみ対応)。
        // 最初の子の上マージンは、親のcontent開始位置にそのまま加算されるはず。
        let dom = html::parse(br#"<div class="outer"><p class="inner">x</p></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet(".outer { margin: 0; } .inner { height: 20px; margin: 12px 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let p = find_laid_out(&laid, ps[0]).expect("p not found");

        assert_eq!(
            p.layout.margin.top, 12.0,
            "the child's own top margin should still apply in full (no parent-child collapsing)"
        );
    }

    #[test]
    fn auto_height_block_sizes_to_children_content() {
        let dom = html::parse(br#"<div class="outer"><p class="inner">x</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".inner { height: 40px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let outer = find_laid_out(&laid, divs[0]).expect("outer div not found");

        assert_eq!(outer.layout.content.height, 40.0);
    }

    #[test]
    fn wrapped_inline_content_drives_auto_height() {
        // 十分な幅があれば1行、狭ければ複数行に折り返される。
        let dom = html::parse(br#"<p class="a">hello world</p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);

        let wide = layout_document(&tree, &styles, &fonts, 800.0);
        let p_wide = find_laid_out(&wide, ps[0]).expect("p not found");
        let LaidOutContent::Inline(lines_wide) = &p_wide.content else {
            panic!("expected inline content")
        };
        assert_eq!(lines_wide.len(), 1);

        let narrow = layout_document(&tree, &styles, &fonts, 60.0);
        let p_narrow = find_laid_out(&narrow, ps[0]).expect("p not found");
        let LaidOutContent::Inline(lines_narrow) = &p_narrow.content else {
            panic!("expected inline content")
        };
        assert_eq!(lines_narrow.len(), 2);

        assert!(p_narrow.layout.content.height > p_wide.layout.content.height);
    }

    #[test]
    fn padding_and_border_offset_content_box() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { width: 100px; margin: 0; padding: 5px; border: 2px solid black; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 100.0);
        assert_eq!(div_box.layout.padding.left, 5.0);
        assert_eq!(div_box.layout.border.left, 2.0);

        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 2.0 + 5.0 + 100.0 + 5.0 + 2.0);
    }

    #[test]
    fn border_style_none_zeroes_out_the_used_border_width_in_layout() {
        // CSS2.1 8.5.3: border-styleがnoneの辺は、border-widthの指定に関わらず
        // 使用値が0になる(枠線が描画されないだけでなく、レイアウト上の
        // 幅計算にも影響しない)。
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { width: 100px; margin: 0; border-width: 5px; border-style: none; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.border.left, 0.0);
        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 100.0);
    }

    #[test]
    fn fragmentation_hints_reflect_the_elements_computed_style() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { break-before: always; break-inside: avoid; orphans: 3; widows: 4; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(
            div_box.fragmentation.break_before,
            super::BreakBetween::Always
        );
        assert_eq!(div_box.fragmentation.break_after, super::BreakBetween::Auto);
        assert_eq!(
            div_box.fragmentation.break_inside,
            super::BreakInside::Avoid
        );
        assert_eq!(div_box.fragmentation.orphans, 3);
        assert_eq!(div_box.fragmentation.widows, 4);
    }

    #[test]
    fn anonymous_boxes_get_default_fragmentation_hints() {
        // 無名ボックス(混在コンテンツの折り返し等)は対応するDOM要素を持たないため、
        // fragmentationヒントは常に初期値(auto/auto/auto/2/2)になるはず。
        let dom = html::parse(br#"<div class="outer">before <p>P</p> after</div>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");
        let LaidOutContent::Blocks(children) = &div_box.content else {
            panic!("expected block container")
        };
        let anonymous = children
            .iter()
            .find(|c| c.node.is_none())
            .expect("expected an anonymous block wrapping the loose text");

        assert_eq!(anonymous.fragmentation, FragmentationHints::default());
    }
}
