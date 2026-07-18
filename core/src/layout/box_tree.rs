//! DOM+計算スタイルからレイアウトボックスツリーを構築する。
//!
//! `display: none`の要素(とその部分木)は除外する。ブロックコンテナの子が
//! block-levelとinline-level/テキストの混在になる場合は、CSSの無名ボックス生成
//! 規則(CSS2.1 9.2.1.1)に従い、連続するinline-levelの内容を無名ブロックボックスに
//! まとめる。無名ボックスは対応するDOMノードを持たないため`node: None`とする。
//!
//! インライン要素内部の構造(装飾・行分割)はT6の責務。ここではテキストを
//! 平坦化するところまでを行う。

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
    /// インラインフォーマッティングコンテキストの内容(平坦化したテキスト)。
    Inline(String),
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
        let mut text = String::new();
        for &child in &child_ids {
            if child_kind(dom, styles, child) == ChildKind::Inline {
                collect_text(dom, child, &mut text);
            }
        }
        BoxContent::Inline(text)
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
    let mut pending_text = String::new();

    for &child in child_ids {
        match child_kind(dom, styles, child) {
            ChildKind::None => {}
            ChildKind::Block => {
                flush_pending_text(&mut pending_text, &mut result);
                if let Some(b) = build_box_for_element(dom, styles, child) {
                    result.push(b);
                }
            }
            ChildKind::Inline => collect_text(dom, child, &mut pending_text),
        }
    }
    flush_pending_text(&mut pending_text, &mut result);

    result
}

fn flush_pending_text(pending: &mut String, result: &mut Vec<LayoutBox>) {
    if !pending.trim().is_empty() {
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

/// インライン要素の子孫を再帰的に辿り、テキストを連結する。
fn collect_text(dom: &Dom, node: NodeId, out: &mut String) {
    match &dom.node(node).data {
        NodeData::Text { contents } => out.push_str(contents),
        NodeData::Element { .. } => {
            for child in dom.children(node) {
                collect_text(dom, child, out);
            }
        }
        _ => {}
    }
}
