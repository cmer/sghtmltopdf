//! [`Dom`]のノードに対する`selectors::Element`実装。

use html5ever::Namespace;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::matching::{ElementSelectorFlags, MatchingContext};
use selectors::{Element, OpaqueElement};

use crate::html::{Dom, Node, NodeData, NodeId};

use super::selector_impl::{CssLocalName, NonTSPseudoClass, PseudoElement, SgSelectorImpl};

#[derive(Clone, Copy)]
pub struct ElementRef<'a> {
    dom: &'a Dom,
    id: NodeId,
}

impl<'a> ElementRef<'a> {
    pub fn new(dom: &'a Dom, id: NodeId) -> Self {
        Self { dom, id }
    }

    pub fn id(&self) -> NodeId {
        self.id
    }

    fn node(&self) -> &'a Node {
        self.dom.node(self.id)
    }

    fn is_element(id: NodeId, dom: &Dom) -> bool {
        matches!(dom.node(id).data, NodeData::Element { .. })
    }
}

impl std::fmt::Debug for ElementRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ElementRef({:?})", self.id)
    }
}

/// 要素の直前/直後にある、要素ノードだけを辿るイテレータ。
fn sibling_elements(dom: &Dom, mut next: Option<NodeId>, forward: bool) -> Option<NodeId> {
    while let Some(id) = next {
        if ElementRef::is_element(id, dom) {
            return Some(id);
        }
        let node = dom.node(id);
        next = if forward {
            node.next_sibling
        } else {
            node.previous_sibling
        };
    }
    None
}

impl<'a> Element for ElementRef<'a> {
    type Impl = SgSelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.node())
    }

    fn parent_element(&self) -> Option<Self> {
        self.dom
            .parent(self.id)
            .map(|id| ElementRef::new(self.dom, id))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        sibling_elements(self.dom, self.node().previous_sibling, false)
            .map(|id| ElementRef::new(self.dom, id))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        sibling_elements(self.dom, self.node().next_sibling, true)
            .map(|id| ElementRef::new(self.dom, id))
    }

    fn first_element_child(&self) -> Option<Self> {
        sibling_elements(self.dom, self.node().first_child, true)
            .map(|id| ElementRef::new(self.dom, id))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        matches!(&self.node().data, NodeData::Element { name, .. } if name.ns == html5ever::ns!(html))
    }

    fn has_local_name(&self, local_name: &CssLocalName) -> bool {
        matches!(&self.node().data, NodeData::Element { name, .. } if name.local == local_name.0)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        matches!(&self.node().data, NodeData::Element { name, .. } if &name.ns == ns)
    }

    fn is_same_type(&self, other: &Self) -> bool {
        match (&self.node().data, &other.node().data) {
            (NodeData::Element { name: a, .. }, NodeData::Element { name: b, .. }) => a == b,
            _ => false,
        }
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&Namespace>,
        local_name: &CssLocalName,
        operation: &AttrSelectorOperation<&super::selector_impl::CssString>,
    ) -> bool {
        let NodeData::Element { attrs, .. } = &self.node().data else {
            return false;
        };
        attrs.iter().any(|attr| {
            !matches!(*ns, NamespaceConstraint::Specific(url) if *url != attr.name.ns)
                && local_name.0 == attr.name.local
                && operation.eval_str(&attr.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        _pc: &NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn match_pseudo_element(
        &self,
        _pe: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        self.has_local_name(&CssLocalName::from("a"))
            && self.has_attr_in_no_namespace(&"href".into())
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &CssLocalName, case_sensitivity: CaseSensitivity) -> bool {
        let NodeData::Element { attrs, .. } = &self.node().data else {
            return false;
        };
        attrs
            .iter()
            .find(|attr| &*attr.name.local == "id")
            .is_some_and(|attr| case_sensitivity.eq(id.0.as_bytes(), attr.value.as_bytes()))
    }

    fn has_class(&self, name: &CssLocalName, case_sensitivity: CaseSensitivity) -> bool {
        let NodeData::Element { attrs, .. } = &self.node().data else {
            return false;
        };
        attrs
            .iter()
            .find(|attr| &*attr.name.local == "class")
            .is_some_and(|attr| {
                attr.value
                    .split_ascii_whitespace()
                    .any(|class| case_sensitivity.eq(name.0.as_bytes(), class.as_bytes()))
            })
    }

    fn has_custom_state(&self, _name: &CssLocalName) -> bool {
        false
    }

    fn imported_part(&self, _name: &CssLocalName) -> Option<CssLocalName> {
        None
    }

    fn is_part(&self, _name: &CssLocalName) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.dom.children(self.id).all(|child| {
            !matches!(
                &self.dom.node(child).data,
                NodeData::Element { .. } | NodeData::Text { .. }
            )
        })
    }

    fn is_root(&self) -> bool {
        self.dom
            .parent(self.id)
            .is_some_and(|parent| matches!(self.dom.node(parent).data, NodeData::Document))
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse;
    use selectors::attr::AttrSelectorOperation;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    /// [`Dom::release_subtree`](crate::html::Dom::release_subtree)で解放済み
    /// のノードは、`NodeData::Released`が既存のいずれの`match`パターンにも
    /// 積極的にマッチしないため、要素として振る舞わなくなる(タグ名・属性・
    /// クラス等の照会がすべて非マッチになる)ことを確認する。これは
    /// [0006](../../docs/decisions/0006-css-non-locality-scope.md)が
    /// 前提とする「解放済みノードは以後のセレクタマッチングで安全に無視
    /// される」という性質の裏付け。
    #[test]
    fn released_node_no_longer_behaves_like_an_element() {
        let mut dom = parse(br#"<div id="x" class="c"><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        dom.release_subtree(div);

        let el = ElementRef::new(&dom, div);
        assert!(!el.has_local_name(&"div".into()));
        assert!(!el.has_id(&"x".into(), selectors::attr::CaseSensitivity::CaseSensitive));
        assert!(!el.has_class(&"c".into(), selectors::attr::CaseSensitivity::CaseSensitive));
        assert!(!el.attr_matches(
            &selectors::attr::NamespaceConstraint::Any,
            &"id".into(),
            &AttrSelectorOperation::Exists,
        ));
    }
}
