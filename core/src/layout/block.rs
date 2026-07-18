//! Block Formatting Context: containing blockに基づく幅計算と、
//! ブロック要素の縦積み配置(CSS2.1 §10.3.3, §9.4.1の簡略版)。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - 隣接兄弟間のマージン相殺(margin collapsing)は行わない
//! - 幅・水平マージンがすべて明示指定された場合の再調整(over-constrained時の
//!   margin-right再計算)は行わない
//! - 高さのパーセンテージ指定はcontaining blockの高さが不定なため`auto`として扱う
//! - インラインコンテンツの高さは行分割前の暫定値(T6で本実装に置き換える)

use std::collections::HashMap;

use crate::html::NodeId;
use crate::style::{ComputedStyle, Display, LengthPercentage, LengthPercentageOrAuto};

use super::box_tree::{BoxContent, LayoutBox};
use super::geometry::{EdgeSizes, Layout, Rect};

#[derive(Debug, Clone)]
pub struct LaidOutBox {
    pub node: Option<NodeId>,
    pub layout: Layout,
    pub content: LaidOutContent,
}

#[derive(Debug, Clone)]
pub enum LaidOutContent {
    Blocks(Vec<LaidOutBox>),
    Inline(String),
}

/// ページ幅を初期containing blockとして、ボックスツリー全体をレイアウトする。
pub fn layout_document(
    root: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    page_width: f32,
) -> LaidOutBox {
    layout_box(root, styles, page_width, 0.0, 0.0)
}

fn layout_box(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    containing_width: f32,
    x: f32,
    y: f32,
) -> LaidOutBox {
    let style = box_style(b, styles);

    let padding = resolve_padding(&style, containing_width);
    let border = resolve_border(&style);
    let (content_width, margin_left, margin_right) = resolve_width_and_horizontal_margins(
        &style,
        containing_width,
        padding.left + padding.right,
        border.left + border.right,
    );
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
            let mut laid_children = Vec::with_capacity(children.len());
            for child in children {
                let child_laid = layout_box(child, styles, content_width, content_x, cursor_y);
                cursor_y += child_laid.layout.margin_box_height();
                laid_children.push(child_laid);
            }
            let auto_height = cursor_y - content_y;
            let height = resolve_height(&style).unwrap_or(auto_height);
            (LaidOutContent::Blocks(laid_children), height)
        }
        BoxContent::Inline(text) => {
            let height =
                resolve_height(&style).unwrap_or_else(|| estimate_inline_height(&style, text));
            (LaidOutContent::Inline(text.clone()), height)
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
        },
        content,
    }
}

fn box_style(b: &LayoutBox, styles: &HashMap<NodeId, ComputedStyle>) -> ComputedStyle {
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

fn resolve_lpa_or_zero(lpa: LengthPercentageOrAuto, basis: f32) -> f32 {
    match lpa {
        LengthPercentageOrAuto::Auto => 0.0,
        LengthPercentageOrAuto::LengthPercentage(lp) => resolve_lp(lp, basis),
    }
}

fn resolve_padding(style: &ComputedStyle, basis: f32) -> EdgeSizes {
    EdgeSizes {
        top: resolve_lp(style.padding_top, basis),
        right: resolve_lp(style.padding_right, basis),
        bottom: resolve_lp(style.padding_bottom, basis),
        left: resolve_lp(style.padding_left, basis),
    }
}

fn resolve_border(style: &ComputedStyle) -> EdgeSizes {
    EdgeSizes {
        top: style.border_top_width.0,
        right: style.border_right_width.0,
        bottom: style.border_bottom_width.0,
        left: style.border_left_width.0,
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

/// CSS2.1 §10.3.3(block-level, non-replaced要素)の簡略版。
/// `margin-left + border-left + padding-left + width + padding-right + border-right + margin-right
/// = containing blockの幅`という制約から、`auto`な項目を埋める。
fn resolve_width_and_horizontal_margins(
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
            // over-constrained: margin-rightの再調整は行わない(簡略化)。
            let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
            let margin_right = resolve_lpa_or_zero(style.margin_right, containing_width);
            (width, margin_left, margin_right)
        }
    }
}

/// T6で行分割を実装するまでの暫定値: テキストが存在すれば1行分の高さとして扱う。
fn estimate_inline_height(style: &ComputedStyle, text: &str) -> f32 {
    if text.trim().is_empty() {
        0.0
    } else {
        style.font_size.0 * 1.2
    }
}

#[cfg(test)]
mod tests {
    use super::super::box_tree::build_box_tree;
    use super::*;
    use crate::html::{self, Dom, NodeData};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

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
        assert!(matches!(&children[0].content, BoxContent::Inline(t) if t.trim() == "before"));
        assert_eq!(children[1].node, Some(ps[0]));
        assert!(matches!(&children[2].content, BoxContent::Inline(t) if t.trim() == "after"));
    }

    #[test]
    fn auto_width_fills_containing_block_minus_margins() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".box { margin: 10px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let laid = layout_document(&tree, &styles, 800.0);

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
        let laid = layout_document(&tree, &styles, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 400.0);
        assert_eq!(div_box.layout.margin.left, div_box.layout.margin.right);
        assert_eq!(div_box.layout.margin.left, 192.0);
    }

    #[test]
    fn block_siblings_stack_vertically_by_content_height() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet(".a { height: 50px; margin: 0; } .b { height: 30px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let laid = layout_document(&tree, &styles, 800.0);

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
    fn auto_height_block_sizes_to_children_content() {
        let dom = html::parse(br#"<div class="outer"><p class="inner">x</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".inner { height: 40px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let laid = layout_document(&tree, &styles, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let outer = find_laid_out(&laid, divs[0]).expect("outer div not found");

        assert_eq!(outer.layout.content.height, 40.0);
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
        let laid = layout_document(&tree, &styles, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 100.0);
        assert_eq!(div_box.layout.padding.left, 5.0);
        assert_eq!(div_box.layout.border.left, 2.0);

        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 2.0 + 5.0 + 100.0 + 5.0 + 2.0);
    }
}
