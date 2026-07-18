//! html5everの`TreeSink`実装。パース結果を[`Dom`]に組み立てる。

use std::cell::{Cell, RefCell};

use html5ever::interface::tree_builder::{
    ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink,
};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{parse_document, Attribute, LocalName, Namespace, QualName};

use super::dom::{append, detach, insert_before, Dom, Node, NodeData, NodeId};

/// HTMLバイト列をパースして[`Dom`]を構築する。
pub fn parse(html: &[u8]) -> Dom {
    let sink = Sink {
        nodes: RefCell::new(vec![Node::new(NodeData::Document)]),
        document: NodeId(0),
        quirks_mode: Cell::new(QuirksMode::NoQuirks),
    };

    parse_document(sink, Default::default())
        .from_utf8()
        .one(html)
}

struct Sink {
    nodes: RefCell<Vec<Node>>,
    document: NodeId,
    quirks_mode: Cell<QuirksMode>,
}

/// [`TreeSink::elem_name`]が返す、貸し出し元から独立した要素名。
///
/// アリーナは1つの`RefCell`にまとめているため、`&'a QualName`のように
/// 借用をそのまま返すことができない(borrowガードの寿命が合わない)。
/// `QualName`は内部がアトム(参照カウント)なのでクローンのコストは小さい。
#[derive(Debug)]
struct OwnedElemName(QualName);

impl ElemName for OwnedElemName {
    fn ns(&self) -> &Namespace {
        &self.0.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

impl Sink {
    fn alloc(&self, data: NodeData) -> NodeId {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(Node::new(data));
        NodeId(nodes.len() - 1)
    }

    /// テキストは直前の兄弟がTextノードであれば連結する(html5everの規約通り)。
    fn append_common(
        &self,
        child: NodeOrText<NodeId>,
        previous_sibling: impl FnOnce(&[Node]) -> Option<NodeId>,
        do_append: impl FnOnce(&mut [Node], NodeId),
    ) {
        let mut nodes = self.nodes.borrow_mut();

        let new_node = match child {
            NodeOrText::AppendText(text) => {
                if let Some(prev) = previous_sibling(&nodes) {
                    if let NodeData::Text { contents } = &mut nodes[prev.0].data {
                        contents.push_str(&text);
                        return;
                    }
                }
                let mut nodes_for_alloc = nodes;
                nodes_for_alloc.push(Node::new(NodeData::Text {
                    contents: text.to_string(),
                }));
                let id = NodeId(nodes_for_alloc.len() - 1);
                do_append(&mut nodes_for_alloc, id);
                return;
            }
            NodeOrText::AppendNode(id) => id,
        };

        do_append(&mut nodes, new_node);
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Dom;
    type ElemName<'a> = OwnedElemName;

    fn finish(self) -> Dom {
        Dom {
            nodes: self.nodes.into_inner(),
            document: self.document,
        }
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        self.document
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> OwnedElemName {
        let nodes = self.nodes.borrow();
        match &nodes[target.0].data {
            NodeData::Element { name, .. } => OwnedElemName(name.clone()),
            _ => panic!("not an element!"),
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let template_contents = if flags.template {
            Some(self.alloc(NodeData::Document))
        } else {
            None
        };
        self.alloc(NodeData::Element {
            name,
            attrs,
            template_contents,
        })
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.alloc(NodeData::Comment {
            contents: text.to_string(),
        })
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.alloc(NodeData::ProcessingInstruction {
            target: target.to_string(),
            contents: data.to_string(),
        })
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        let parent = *parent;
        self.append_common(
            child,
            |nodes| nodes[parent.0].last_child,
            |nodes, new_node| append(nodes, parent, new_node),
        );
    }

    fn append_before_sibling(&self, sibling: &NodeId, child: NodeOrText<NodeId>) {
        let sibling = *sibling;
        self.append_common(
            child,
            |nodes| nodes[sibling.0].previous_sibling,
            |nodes, new_node| insert_before(nodes, sibling, new_node),
        );
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.nodes.borrow()[element.0].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let doctype = self.alloc(NodeData::Doctype {
            name: name.to_string(),
        });
        append(&mut self.nodes.borrow_mut(), self.document, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        match &self.nodes.borrow()[target.0].data {
            NodeData::Element {
                template_contents: Some(contents),
                ..
            } => *contents,
            _ => panic!("not a template element!"),
        }
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        let NodeData::Element {
            attrs: existing, ..
        } = &mut nodes[target.0].data
        else {
            panic!("not an element");
        };
        for attr in attrs {
            if !existing.iter().any(|a| a.name == attr.name) {
                existing.push(attr);
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        detach(&mut self.nodes.borrow_mut(), *target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut nodes = self.nodes.borrow_mut();
        let mut next_child = nodes[node.0].first_child;
        while let Some(child) = next_child {
            next_child = nodes[child.0].next_sibling;
            append(&mut nodes, *new_parent, child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 木を先行順(pre-order)に探索し、最初に見つかったタグ名の要素を返す。
    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    fn text_of(dom: &Dom, id: NodeId) -> String {
        let mut out = String::new();
        collect_text(dom, id, &mut out);
        out
    }

    fn collect_text(dom: &Dom, id: NodeId, out: &mut String) {
        if let NodeData::Text { contents } = &dom.node(id).data {
            out.push_str(contents);
        }
        for child in dom.children(id) {
            collect_text(dom, child, out);
        }
    }

    #[test]
    fn parses_element_tree_with_attrs_and_text() {
        let dom = parse(br#"<div class="a"><p>Hello <b>world</b></p></div>"#);

        let div = find(&dom, dom.document(), "div").expect("div not found");
        let NodeData::Element { attrs, .. } = &dom.node(div).data else {
            panic!("expected element")
        };
        assert_eq!(attrs.len(), 1);
        assert_eq!(&*attrs[0].name.local, "class");
        assert_eq!(&*attrs[0].value, "a");

        let p = find(&dom, div, "p").expect("p not found");
        assert_eq!(text_of(&dom, p), "Hello world");

        let b = find(&dom, p, "b").expect("b not found");
        assert_eq!(text_of(&dom, b), "world");
    }

    #[test]
    fn merges_adjacent_text_into_a_single_node() {
        // "&amp;"はトークナイザ内で文字参照として個別に処理されるため、
        // 素朴に実装すると隣接テキストノードが分裂しやすい典型ケース。
        let dom = parse(br#"<p>AT&amp;T</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let children: Vec<_> = dom.children(p).collect();
        assert_eq!(
            children.len(),
            1,
            "adjacent text nodes should be merged into one"
        );
        assert_eq!(text_of(&dom, p), "AT&T");
    }

    #[test]
    fn parses_sibling_elements_in_order() {
        let dom = parse(br#"<ul><li>one</li><li>two</li><li>three</li></ul>"#);
        let ul = find(&dom, dom.document(), "ul").expect("ul not found");

        let lis: Vec<_> = dom.children(ul).collect();
        assert_eq!(lis.len(), 3);
        assert_eq!(text_of(&dom, lis[0]), "one");
        assert_eq!(text_of(&dom, lis[1]), "two");
        assert_eq!(text_of(&dom, lis[2]), "three");
    }
}
