//! DOM+計算スタイルからレイアウトボックスツリーを構築する。
//!
//! `display: none`の要素(とその部分木)は除外する。ブロックコンテナの子が
//! block-levelとinline-level/テキストの混在になる場合は、CSSの無名ボックス生成
//! 規則(CSS2.1 9.2.1.1)に従い、連続するinline-levelの内容を無名ブロックボックスに
//! まとめる。無名ボックスは対応するDOMノードを持たないため`node: None`とする。
//!
//! インライン要素内部の行分割・実際の描画はT6の責務。ここでは、`<b>`/`<span>`
//! 等のインライン要素境界をテキストノード単位の[`InlineSpan`]として保持したまま
//! 平坦化する(要素そのものを畳み込みはするが、どの計算スタイルが適用される
//! テキストかという情報は失わない)。

use std::collections::HashMap;

use crate::html::{Dom, NodeData, NodeId};
use crate::style::{ComputedStyle, Display};

#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// 対応するDOM要素。無名ボックスの場合は`None`。
    pub node: Option<NodeId>,
    pub content: BoxContent,
}

#[derive(Debug, Clone)]
pub enum BoxContent {
    Blocks(Vec<LayoutBox>),
    /// インラインフォーマッティングコンテキストの内容。
    Inline(Vec<InlineSpan>),
}

/// 1つのDOMテキストノードに由来する、単一の計算スタイルを持つテキスト区間。
#[derive(Debug, Clone)]
pub struct InlineSpan {
    /// このテキストの元になったDOMテキストノード。`styles`から計算スタイルを
    /// 引く(`<b>`/`<span style="...">`等の祖先の宣言は、テキストノード自身の
    /// 計算スタイルに継承・カスケード済みなので、このノードのスタイルを見れば足りる)。
    pub node: NodeId,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildKind {
    /// `display: none`、空白のみのテキストなど、ボックスを生成しない。
    None,
    Block,
    Inline,
}

pub fn build_box_tree(dom: &Dom, styles: &HashMap<NodeId, ComputedStyle>) -> LayoutBox {
    let child_ids: Vec<NodeId> = dom.children(dom.document()).collect();
    LayoutBox {
        node: None,
        content: BoxContent::Blocks(build_children_boxes(dom, styles, &child_ids)),
    }
}

fn build_box_for_element(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
) -> Option<LayoutBox> {
    let style = styles.get(&node)?;
    if style.display == Display::None {
        return None;
    }

    let child_ids: Vec<NodeId> = dom.children(node).collect();
    let has_block_child = child_ids
        .iter()
        .any(|&c| child_kind(dom, styles, c) == ChildKind::Block);

    let content = if has_block_child {
        BoxContent::Blocks(build_children_boxes(dom, styles, &child_ids))
    } else {
        let mut spans = Vec::new();
        for &child in &child_ids {
            if child_kind(dom, styles, child) == ChildKind::Inline {
                collect_spans(dom, child, &mut spans);
            }
        }
        BoxContent::Inline(spans)
    };

    Some(LayoutBox {
        node: Some(node),
        content,
    })
}

fn build_children_boxes(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    child_ids: &[NodeId],
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    let mut pending_spans: Vec<InlineSpan> = Vec::new();

    for &child in child_ids {
        match child_kind(dom, styles, child) {
            ChildKind::None => {}
            ChildKind::Block => {
                flush_pending_spans(&mut pending_spans, &mut result);
                if let Some(b) = build_box_for_element(dom, styles, child) {
                    result.push(b);
                }
            }
            ChildKind::Inline => collect_spans(dom, child, &mut pending_spans),
        }
    }
    flush_pending_spans(&mut pending_spans, &mut result);

    result
}

fn flush_pending_spans(pending: &mut Vec<InlineSpan>, result: &mut Vec<LayoutBox>) {
    if !pending.iter().all(|span| span.text.trim().is_empty()) {
        result.push(LayoutBox {
            node: None,
            content: BoxContent::Inline(std::mem::take(pending)),
        });
    }
    pending.clear();
}

fn child_kind(dom: &Dom, styles: &HashMap<NodeId, ComputedStyle>, node: NodeId) -> ChildKind {
    match &dom.node(node).data {
        NodeData::Element { .. } => match styles.get(&node).map(|s| s.display) {
            Some(Display::None) | None => ChildKind::None,
            Some(Display::Block) => ChildKind::Block,
            Some(Display::Inline) => ChildKind::Inline,
        },
        NodeData::Text { contents } => {
            if contents.trim().is_empty() {
                ChildKind::None
            } else {
                ChildKind::Inline
            }
        }
        _ => ChildKind::None,
    }
}

/// インライン要素の子孫を再帰的に辿り、テキストノードごとに[`InlineSpan`]を積む。
/// テキストノード自身の計算スタイルに、祖先のインライン要素(`<b>`/`<span>`等)の
/// カスケード・継承結果が反映済みのため、ここではノードIDを保持するだけでよい。
fn collect_spans(dom: &Dom, node: NodeId, out: &mut Vec<InlineSpan>) {
    match &dom.node(node).data {
        NodeData::Text { contents } => out.push(InlineSpan {
            node,
            text: contents.clone(),
        }),
        NodeData::Element { .. } => {
            for child in dom.children(node) {
                collect_spans(dom, child, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use crate::style::{compute_styles, user_agent_stylesheet, RgbaColor, Stylesheet};

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    fn find_inline_spans(b: &LayoutBox) -> Option<&Vec<InlineSpan>> {
        match &b.content {
            BoxContent::Inline(spans) => Some(spans),
            BoxContent::Blocks(children) => children.iter().find_map(find_inline_spans),
        }
    }

    #[test]
    fn inline_element_boundaries_are_preserved_as_separate_spans() {
        let dom = html::parse(br#"<p>before <b>bold</b> after</p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let p = find(&dom, dom.document(), "p").expect("p not found");
        let b = find(&dom, p, "b").expect("b not found");
        let p_box = find_box(&tree, p).expect("p box not found");
        let spans = find_inline_spans(p_box).expect("expected inline content");

        assert_eq!(spans.len(), 3, "before-text / bold-text / after-text");
        assert_eq!(spans[0].text, "before ");
        assert_eq!(spans[1].text, "bold");
        assert_eq!(spans[2].text, " after");
        // 太字テキストのスパンは<b>の子テキストノード由来であり、<p>直下のテキストとは
        // 別のNodeIdを持つ(=別の計算スタイルを引ける)。
        assert_ne!(spans[0].node, spans[1].node);
        assert_eq!(dom.children(b).next(), Some(spans[1].node));
    }

    #[test]
    fn span_style_reflects_ancestor_cascade_at_layout_time() {
        let dom = html::parse(br#"<p>plain <b style="color: rgb(9, 9, 9);">loud</b></p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let p = find(&dom, dom.document(), "p").expect("p not found");
        let p_box = find_box(&tree, p).expect("p box not found");
        let spans = find_inline_spans(p_box).expect("expected inline content");

        let loud_style = &styles[&spans[1].node];
        assert_eq!(
            loud_style.color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            }
        );
        assert_eq!(loud_style.font_weight, crate::style::FontWeight::Bold);
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
}
