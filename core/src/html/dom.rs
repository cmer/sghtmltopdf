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
    /// ルート([`Dom::document`])からの深さ。木に繋がれた時点で確定し、
    /// 別の親へ移し替えられれば部分木ごと振り直される
    /// ([`set_subtree_depth`])。深さ上限の判定に使う。
    pub(crate) depth: u32,
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
            depth: 0,
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
    /// [`Dom::release_subtree`]で解放済みのノード。
    ///
    /// テキスト内容・属性など重いデータは破棄済みだが、`Node`の
    /// `parent`/`previous_sibling`/`next_sibling`/`first_child`/`last_child`
    /// (木構造のリンク)はそのまま残す。`NodeId`はアリーナ(`Vec<Node>`)の
    /// 固定インデックスであり、要素を実際に取り除くとインデックスがずれて
    /// 他の`NodeId`が指す先を壊してしまうため、ノードのスロット自体は
    /// 削除せず中身だけを空にする「タブストーン化」方式を採る。
    ///
    /// 既存の`NodeData`パターンマッチ(`if let NodeData::Element {..}`等)
    /// はいずれも網羅的ではなく`Released`をワイルドカード側で自然に扱うため、
    /// 「要素でもテキストでもない」として黙って無視される。これは安全側
    /// (誤ってマッチしない)に倒れるが、逆に「本来解放してはいけないノードを
    /// 誤って解放してしまった」というバグはサイレントな不具合として現れうる。
    /// 解放を呼び出すタイミングの安全性(「兄弟・子孫セレクタの参照範囲を
    /// 跨がない」制約)は呼び出し側の責務であり、
    /// この型自体はそれを強制しない。
    Released,
}

/// `<link>`要素が`rel="stylesheet"`かどうかを判定する。
///
/// HTML仕様上`rel`は空白区切りのトークン列(`rel="stylesheet preload"`も
/// 有効)であり、単純な文字列完全一致では`stylesheet`以外のトークンが
/// 混ざったケースを見逃す。`class`属性のトークンマッチング
/// (`style/element_ref.rs`の`has_class`)と同種の注意点。
pub fn is_stylesheet_link(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .find(|attr| &*attr.name.local == "rel")
        .is_some_and(|attr| {
            attr.value
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        })
}

/// 文書内の最初の`<title>`のテキスト(PDF Info辞書の`/Title`用)。`--title`が
/// 指定されていない場合のフォールバックとして使う。
pub fn find_document_title(dom: &Dom) -> Option<String> {
    fn walk(dom: &Dom, node: NodeId) -> Option<String> {
        if let NodeData::Element { name, .. } = &dom.node(node).data {
            if &*name.local == "title" {
                let mut text = String::new();
                for child in dom.children(node) {
                    if let NodeData::Text { contents } = &dom.node(child).data {
                        text.push_str(contents);
                    }
                }
                let text = text.trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        for child in dom.children(node) {
            if let Some(found) = walk(dom, child) {
                return Some(found);
            }
        }
        None
    }
    walk(dom, dom.document())
}

/// 文書内の最初の`<base href>`の値。`<body>`より後に現れた`<base>`は無視する
/// (`Mode::Streaming`では原理的に
/// 反映できないため、両モードで同じ挙動に揃える)。
pub fn find_base_href(dom: &Dom) -> Option<String> {
    fn walk(dom: &Dom, node: NodeId, seen_body: &mut bool) -> Option<String> {
        if let NodeData::Element { name, attrs, .. } = &dom.node(node).data {
            match &*name.local {
                "body" => *seen_body = true,
                "base" if !*seen_body => {
                    let href = attrs
                        .iter()
                        .find(|attr| &*attr.name.local == "href")
                        .map(|attr| attr.value.trim().to_string())
                        .filter(|href| !href.is_empty());
                    if href.is_some() {
                        return href;
                    }
                }
                _ => {}
            }
        }
        for child in dom.children(node) {
            if let Some(found) = walk(dom, child, seen_body) {
                return Some(found);
            }
        }
        None
    }
    let mut seen_body = false;
    walk(dom, dom.document(), &mut seen_body)
}

/// アンカーの対象になりうる要素(`id`属性を持つ要素、および`<a name>`)を
/// `NodeId` → 名前(`id`/`name`の値)として集める。
///
/// 同じ名前が複数回現れた場合はドキュメント順で最初のものを採用する
/// (HTML仕様どおり)。
pub fn collect_anchor_targets(dom: &Dom) -> Vec<(NodeId, String)> {
    fn walk(dom: &Dom, node: NodeId, out: &mut Vec<(NodeId, String)>) {
        if let NodeData::Element { name, attrs, .. } = &dom.node(node).data {
            let is_anchor_element = &*name.local == "a";
            let target = attrs
                .iter()
                .find(|attr| {
                    &*attr.name.local == "id" || (is_anchor_element && &*attr.name.local == "name")
                })
                .map(|attr| attr.value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(target) = target {
                if !out.iter().any(|(_, existing)| *existing == target) {
                    out.push((node, target));
                }
            }
        }
        for child in dom.children(node) {
            walk(dom, child, out);
        }
    }
    let mut out = Vec::new();
    walk(dom, dom.document(), &mut out);
    out
}

/// パース済みのDOM木。
pub struct Dom {
    pub(crate) nodes: Vec<Node>,
    pub(crate) document: NodeId,
    /// この木に現れた最大の深さ。木を組み立てながら更新するため、パース途中
    /// (ストリーミング)でも参照できる。
    pub(crate) max_depth: u32,
    /// まだ内容を保持しているノードの数。
    ///
    /// `nodes`の長さではなく、[`Self::release_subtree`]で解放したぶんを
    /// 差し引いた値。ノードは解放しても`nodes`から取り除かない(NodeIdが
    /// 添字なので詰められない)ため、長さでは実際の保持量を表せない。
    pub(crate) live_nodes: usize,
}

impl Dom {
    pub fn document(&self) -> NodeId {
        self.document
    }

    /// これまでに木へ繋がれたノードの最大深さ([`Node::depth`])。
    ///
    /// DOMを再帰的に辿る処理(スタイル計算・ボックスツリー構築・レイアウト・
    /// PDF描画、および`LayoutBox`の再帰Drop)はいずれも深さに比例して
    /// スタックを消費するため、それらを走らせる前にこの値を上限
    /// ([`crate::html::MAX_ELEMENT_DEPTH`])と比べて拒否する。
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// まだ内容を保持しているノードの数([`Self::live_nodes`])。
    ///
    /// スタイル計算・ボックスツリー・レイアウト結果はこれに比例して増える
    /// ため、メモリの上限判定に使う([`crate::html::MAX_NODES`])。
    pub fn node_count(&self) -> usize {
        self.live_nodes
    }

    /// ノードを1つ足して`NodeId`を返す。
    ///
    /// `nodes`への追加経路をここ1本にまとめ、[`Self::live_nodes`]の更新
    /// 漏れを防ぐ。
    pub(crate) fn push_node(&mut self, data: NodeData) -> NodeId {
        self.nodes.push(Node::new(data));
        self.live_nodes += 1;
        NodeId(self.nodes.len() - 1)
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

    /// `root`以下の部分木を再帰的に解放する。
    ///
    /// 各ノードの`data`を[`NodeData::Released`]に置き換え、テキスト内容・
    /// 属性等の重いデータを破棄する。`root`自身も解放対象に含む。木構造の
    /// リンク(`parent`/`previous_sibling`/`next_sibling`/`first_child`/
    /// `last_child`)は変更しないため、解放後もこの部分木を経由した
    /// ナビゲーション(`children`/`parent`)自体は壊れない。
    ///
    /// 安全に呼べるのは、`root`以下がスタイル計算・レイアウトともに完了し、
    /// かつ以後どの要素のセレクタマッチングからも参照されないことが確定した
    /// 場合のみ(「兄弟・子孫セレクタの参照範囲を跨がない」制約)。この判定は
    /// 呼び出し側の責務で、`Dom`自体はそれを強制しない。
    pub fn release_subtree(&mut self, root: NodeId) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            stack.extend(self.children(id));
            // 二重解放でも数え過ぎないよう、解放済みは飛ばす。
            if !matches!(self.nodes[id.0].data, NodeData::Released) {
                self.live_nodes -= 1;
                self.nodes[id.0].data = NodeData::Released;
            }
        }
    }

    /// `root`の子孫だけを解放し、`root`自身は要素として残す。
    ///
    /// 残った`root`はタグ名・クラス・idを保つので、後続の兄弟からは
    /// 「直前の兄弟」として見え続ける。`+`/`~`や`:first-child`のように
    /// 直前の兄弟が要るセレクタを使う文書では、[`Self::release_subtree`]の
    /// 代わりにこちらを使う(ストリーミング処理での使い分けは
    /// `style::needs_preceding_siblings`が判断する)。
    ///
    /// 残るのはトップレベル要素1個につきノード1個なので、解放できる量は
    /// ほぼ変わらない(子孫が大半を占めるため)。
    pub fn release_descendants(&mut self, root: NodeId) {
        let children: Vec<NodeId> = self.children(root).collect();
        for child in children {
            self.release_subtree(child);
        }
    }

    /// `id`が[`Dom::release_subtree`]で解放済みかどうか。
    pub fn is_released(&self, id: NodeId) -> bool {
        matches!(self.node(id).data, NodeData::Released)
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
/// `root`以下の[`Node::depth`]を`depth`起点で振り直し、部分木内の最大深さを返す。
///
/// 明示スタックで辿る。ここを再帰で書くと、深さ上限を判定するための処理自体が
/// 深いDOMでスタックを溢れさせてしまい本末転倒になる。
pub(crate) fn set_subtree_depth(nodes: &mut [Node], root: NodeId, depth: u32) -> u32 {
    let mut max = depth;
    let mut stack = vec![(root, depth)];
    while let Some((id, d)) = stack.pop() {
        nodes[id.0].depth = d;
        max = max.max(d);
        let mut child = nodes[id.0].first_child;
        while let Some(c) = child {
            stack.push((c, d + 1));
            child = nodes[c.0].next_sibling;
        }
    }
    max
}

/// `child`を`parent`の末尾に繋ぎ、繋いだ部分木の最大深さを返す。
pub(crate) fn append(nodes: &mut [Node], parent: NodeId, child: NodeId) -> u32 {
    detach(nodes, child);

    nodes[child.0].parent = Some(parent);
    if let Some(last) = nodes[parent.0].last_child {
        nodes[child.0].previous_sibling = Some(last);
        nodes[last.0].next_sibling = Some(child);
    } else {
        nodes[parent.0].first_child = Some(child);
    }
    nodes[parent.0].last_child = Some(child);

    set_subtree_depth(nodes, child, nodes[parent.0].depth + 1)
}

/// `new_node`を`sibling`の直前に挿入する(既存の親からは自動的にdetachされる)。
pub(crate) fn insert_before(nodes: &mut [Node], sibling: NodeId, new_node: NodeId) -> u32 {
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

    // 兄弟として並ぶので深さは`sibling`と同じ。
    set_subtree_depth(nodes, new_node, nodes[sibling.0].depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    #[test]
    fn release_subtree_marks_root_and_descendants_as_released() {
        let mut dom = parse(br#"<div><p>Hello <b>world</b></p></div>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let b = find(&dom, p, "b").expect("b not found");

        dom.release_subtree(p);

        assert!(dom.is_released(p), "root of the released subtree");
        assert!(dom.is_released(b), "descendant of the released subtree");
    }

    #[test]
    fn release_subtree_does_not_affect_siblings_or_ancestors() {
        let mut dom = parse(br#"<div><p>first</p><p>second</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let first = dom.children(div).next().expect("first <p> not found");
        let second = dom.children(div).nth(1).expect("second <p> not found");

        dom.release_subtree(first);

        assert!(dom.is_released(first));
        assert!(
            !dom.is_released(second),
            "sibling outside the released subtree must be unaffected"
        );
        assert!(
            !dom.is_released(div),
            "ancestor outside the released subtree must be unaffected"
        );
    }

    #[test]
    fn tree_navigation_still_works_across_a_released_subtree() {
        // 兄弟の1人目が解放済みでも、木構造のリンク自体は保持されるため、
        // 2人目からその親・祖先へのナビゲーションは引き続き機能する。
        let mut dom = parse(br#"<div><p>first</p><p>second</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let first = dom.children(div).next().expect("first <p> not found");
        let second = dom.children(div).nth(1).expect("second <p> not found");

        dom.release_subtree(first);

        assert_eq!(dom.parent(second), Some(div));
        assert_eq!(
            dom.children(div).count(),
            2,
            "child link count is preserved"
        );
    }

    #[test]
    fn find_base_href_returns_the_first_base_in_head() {
        let dom = crate::html::parse(
            br#"<html><head><base href="https://example.com/docs/"><base href="https://other.example/"></head><body>x</body></html>"#,
        );
        assert_eq!(
            find_base_href(&dom).as_deref(),
            Some("https://example.com/docs/")
        );
    }

    #[test]
    fn find_base_href_is_none_without_a_base_element() {
        let dom = crate::html::parse(br#"<html><head></head><body>x</body></html>"#);
        assert!(find_base_href(&dom).is_none());
    }

    #[test]
    fn find_base_href_ignores_a_base_without_href_and_an_empty_href() {
        let dom = crate::html::parse(
            br#"<html><head><base target="_blank"><base href="  "></head><body>x</body></html>"#,
        );
        assert!(find_base_href(&dom).is_none());
    }

    #[test]
    fn find_base_href_ignores_a_base_that_appears_after_body_starts() {
        // `Mode::Streaming`では原理的に反映できないため、両モードで無視する。
        let dom = crate::html::parse(
            br#"<html><body><base href="https://example.com/"><p>x</p></body></html>"#,
        );
        assert!(find_base_href(&dom).is_none());
    }
}
