//! アリーナ(`Vec<Node>`)ベースの最小DOM。
//!
//! ノード間の親子・兄弟関係は`NodeId`(アリーナのインデックス)で表現する。
//! `Rc<RefCell<Node>>`のようなノード単位の参照カウント/借用チェックを避け、
//! 後続フェーズ(スタイル計算・レイアウト)がDOMを気軽に持ち回れるようにする。

use html5ever::{Attribute, QualName};

/// アリーナ内のノードを指すID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

#[derive(Debug)]
pub struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) previous_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub data: NodeData,
}

impl Node {
    pub(crate) fn new(data: NodeData) -> Self {
        Self {
            parent: None,
            previous_sibling: None,
            next_sibling: None,
            first_child: None,
            last_child: None,
            data,
        }
    }
}

#[derive(Debug)]
pub enum NodeData {
    Document,
    Doctype {
        name: String,
    },
    Text {
        contents: String,
    },
    Comment {
        contents: String,
    },
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        /// `<template>`要素の内容を保持する別ドキュメントノード。
        template_contents: Option<NodeId>,
    },
    ProcessingInstruction {
        target: String,
        contents: String,
    },
}

/// パース済みのDOM木。
pub struct Dom {
    pub(crate) nodes: Vec<Node>,
    pub(crate) document: NodeId,
}

impl Dom {
    pub fn document(&self) -> NodeId {
        self.document
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            dom: self,
            next: self.node(id).first_child,
        }
    }
}

pub struct Children<'a> {
    dom: &'a Dom,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.dom.node(current).next_sibling;
        Some(current)
    }
}

/// `id`をその親・兄弟から切り離す。
pub(crate) fn detach(nodes: &mut [Node], id: NodeId) {
    let (parent, previous_sibling, next_sibling) = {
        let node = &mut nodes[id.0];
        (
            node.parent.take(),
            node.previous_sibling.take(),
            node.next_sibling.take(),
        )
    };

    if let Some(next) = next_sibling {
        nodes[next.0].previous_sibling = previous_sibling;
    } else if let Some(parent) = parent {
        nodes[parent.0].last_child = previous_sibling;
    }

    if let Some(previous) = previous_sibling {
        nodes[previous.0].next_sibling = next_sibling;
    } else if let Some(parent) = parent {
        nodes[parent.0].first_child = next_sibling;
    }
}

/// `child`を`parent`の最後の子として追加する(既存の親からは自動的にdetachされる)。
pub(crate) fn append(nodes: &mut [Node], parent: NodeId, child: NodeId) {
    detach(nodes, child);

    nodes[child.0].parent = Some(parent);
    if let Some(last) = nodes[parent.0].last_child {
        nodes[child.0].previous_sibling = Some(last);
        nodes[last.0].next_sibling = Some(child);
    } else {
        nodes[parent.0].first_child = Some(child);
    }
    nodes[parent.0].last_child = Some(child);
}

/// `new_node`を`sibling`の直前に挿入する(既存の親からは自動的にdetachされる)。
pub(crate) fn insert_before(nodes: &mut [Node], sibling: NodeId, new_node: NodeId) {
    detach(nodes, new_node);

    let parent = nodes[sibling.0].parent;
    nodes[new_node.0].parent = parent;
    nodes[new_node.0].next_sibling = Some(sibling);

    let previous = nodes[sibling.0].previous_sibling;
    nodes[new_node.0].previous_sibling = previous;
    if let Some(previous) = previous {
        nodes[previous.0].next_sibling = Some(new_node);
    } else if let Some(parent) = parent {
        nodes[parent.0].first_child = Some(new_node);
    }
    nodes[sibling.0].previous_sibling = Some(new_node);
}
