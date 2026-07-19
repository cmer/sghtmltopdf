//! html5everの`TreeSink`実装。パース結果を[`Dom`]に組み立てる。

use std::cell::{Cell, Ref, RefCell, RefMut};

use html5ever::interface::tree_builder::{
    ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink,
};
use html5ever::tendril::stream::Utf8LossyDecoder;
use html5ever::tendril::{ByteTendril, StrTendril, TendrilSink};
use html5ever::{parse_document, Attribute, LocalName, Namespace, Parser, QualName};

use super::dom::{append, detach, insert_before, Dom, Node, NodeData, NodeId};

/// HTMLバイト列をパースして[`Dom`]を構築する(一括変換)。
///
/// 内部的には[`StreamingParser`]に全バイト列を1回で`feed`するだけの
/// 薄いラッパー。M1由来の一括処理APIとチャンク投入APIでロジックを
/// 共有するための構成。
pub fn parse(html: &[u8]) -> Dom {
    let mut parser = StreamingParser::new();
    parser.feed(html);
    parser.finish()
}

/// HTMLをチャンク単位で逐次投入できるパーサ。
///
/// html5everのトークナイザ自体がストリーミング設計であることに加え、
/// [`Utf8LossyDecoder`](html5ever::driver::Utf8LossyDecoder)は`feed`の
/// 呼び出し境界がUTF-8のマルチバイト文字の途中で分割されても、続きの
/// バイト列と結合してから正しくデコードする(`tendril`クレートの
/// インクリメンタルデコード機構)。そのためチャンクの区切り位置を
/// 呼び出し側がUTF-8境界に合わせる必要はない。
pub struct StreamingParser {
    inner: Utf8LossyDecoder<Parser<Sink>>,
    /// [`Self::take_completed_top_level_children`]がこれまでに返却済みの、
    /// `<body>`直下の最後の子要素。次回呼び出し時、この続きから探索する。
    last_yielded_top_level_child: Option<NodeId>,
}

impl StreamingParser {
    pub fn new() -> Self {
        let sink = Sink {
            dom: RefCell::new(Dom {
                nodes: vec![Node::new(NodeData::Document)],
                document: NodeId(0),
            }),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            seen_body: Cell::new(false),
            late_style_detected: Cell::new(false),
            body_id: Cell::new(None),
        };
        Self {
            inner: parse_document(sink, Default::default()).from_utf8(),
            last_yielded_top_level_child: None,
        }
    }

    /// HTMLバイト列のチャンクを1つ投入する。何度でも呼べる。
    pub fn feed(&mut self, chunk: &[u8]) {
        self.inner.process(ByteTendril::from_slice(chunk));
    }

    /// `<body>`より後に`<style>`要素が出現したかどうか。
    ///
    /// `Engine`の`Mode::Streaming`が
    /// [0006](../../../docs/decisions/0006-css-non-locality-scope.md)の
    /// 方針に従ってエラーを返すかどうかの判定に使う(`Mode::Batch`では
    /// この値を無視してよい)。`<body>`の開始タグを見た時点以降に生成された
    /// `<style>`要素が1つでもあれば`true`になる。
    pub fn has_late_style_tag(&self) -> bool {
        self.sink().late_style_detected.get()
    }

    /// `<body>`要素の`NodeId`(まだパースされていなければ`None`)。
    pub fn body_node(&self) -> Option<NodeId> {
        self.sink().body_id.get()
    }

    /// パース中の(まだ`finish`されていない)[`Dom`]への読み取り専用アクセス。
    ///
    /// `Engine`の真のストリーミング処理が、`<body>`直下のトップレベル要素が
    /// 確定するたびに、`finish`を待たずにそのサブツリーのスタイル計算・
    /// ボックスツリー構築を行うために使う。
    pub fn dom(&self) -> Ref<'_, Dom> {
        self.sink().dom.borrow()
    }

    /// [`Self::dom`]の書き込み可能版。`Engine`が
    /// [`crate::html::Dom::release_subtree`]でトップレベル要素のサブツリーを
    /// 解放するために使う。
    pub fn dom_mut(&self) -> RefMut<'_, Dom> {
        self.sink().dom.borrow_mut()
    }

    /// `<body>`直下の子要素のうち、「もう変更されない」と判断できる
    /// (=直後に別の兄弟が既に追加されている)ものの`NodeId`を、出現順に
    /// 切り出して返す。呼び出しのたびに返却済み位置を進めるため、同じ
    /// 要素が2回返されることはない。`<body>`がまだ存在しない、または
    /// 対象がなければ空のベクタを返す。
    ///
    /// **安全性の前提**: 正しくネストされたHTML(不正なタグの入れ子や、
    /// テーブル内の不正な要素がテーブルの外へ移動する「foster
    /// parenting」等のhtml5everのエラー回復動作が発生しないこと)を想定
    /// する。エラー回復によって`<body>`直下の要素構成が後から変わる
    /// 場合、この判定は不正確になりうる(既知の限界)。
    ///
    /// 末尾の要素は「まだ子要素が追加され続けている可能性がある」ため、
    /// 対象に含めない(次回以降の呼び出し、または[`Self::finish`]まで
    /// 待つ)。
    pub fn take_completed_top_level_children(&mut self) -> Vec<NodeId> {
        let Some(body) = self.body_node() else {
            return Vec::new();
        };

        let dom = self.dom();
        let mut children: Vec<NodeId> = dom.children(body).collect();
        drop(dom);

        if children.len() < 2 {
            // 末尾以外に確定した要素がない(0〜1個しかない)。
            return Vec::new();
        }

        let start = match self.last_yielded_top_level_child {
            Some(last) => match children.iter().position(|&id| id == last) {
                Some(i) => i + 1,
                None => 0,
            },
            None => 0,
        };
        // 最後の1要素は「まだ子要素が追加中かもしれない」ため除外する。
        let end = children.len() - 1;
        if start >= end {
            return Vec::new();
        }

        children.truncate(end);
        let result = children.split_off(start);
        if let Some(&last) = result.last() {
            self.last_yielded_top_level_child = Some(last);
        }
        result
    }

    /// [`Self::take_completed_top_level_children`]と同様だが、末尾の要素も
    /// 保留せずすべて返す。「これ以上`<body>`に子要素が追加されない」ことが
    /// 確定した状況(`Engine::finish`が呼ばれる直前)で使う。
    pub fn take_all_remaining_top_level_children(&mut self) -> Vec<NodeId> {
        let Some(body) = self.body_node() else {
            return Vec::new();
        };
        let dom = self.dom();
        let children: Vec<NodeId> = dom.children(body).collect();
        drop(dom);

        let start = match self.last_yielded_top_level_child {
            Some(last) => children
                .iter()
                .position(|&id| id == last)
                .map(|i| i + 1)
                .unwrap_or(0),
            None => 0,
        };
        let result = children[start..].to_vec();
        self.last_yielded_top_level_child = children.last().copied();
        result
    }

    fn sink(&self) -> &Sink {
        &self.inner.inner_sink.tokenizer.sink.sink
    }

    /// これ以上チャンクがないことを伝え、パース済みの[`Dom`]を得る。
    pub fn finish(self) -> Dom {
        self.inner.finish()
    }
}

impl Default for StreamingParser {
    fn default() -> Self {
        Self::new()
    }
}

struct Sink {
    dom: RefCell<Dom>,
    quirks_mode: Cell<QuirksMode>,
    /// `<body>`要素の開始タグを見たかどうか([`StreamingParser::has_late_style_tag`]参照)。
    seen_body: Cell<bool>,
    /// `<body>`より後に`<style>`要素が出現したかどうか。
    late_style_detected: Cell<bool>,
    /// `<body>`要素の`NodeId`([`StreamingParser::body_node`]参照)。
    body_id: Cell<Option<NodeId>>,
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
        let mut dom = self.dom.borrow_mut();
        dom.nodes.push(Node::new(data));
        NodeId(dom.nodes.len() - 1)
    }

    /// テキストは直前の兄弟がTextノードであれば連結する(html5everの規約通り)。
    fn append_common(
        &self,
        child: NodeOrText<NodeId>,
        previous_sibling: impl FnOnce(&[Node]) -> Option<NodeId>,
        do_append: impl FnOnce(&mut [Node], NodeId),
    ) {
        let mut dom = self.dom.borrow_mut();

        let new_node = match child {
            NodeOrText::AppendText(text) => {
                if let Some(prev) = previous_sibling(&dom.nodes) {
                    if let NodeData::Text { contents } = &mut dom.nodes[prev.0].data {
                        contents.push_str(&text);
                        return;
                    }
                }
                dom.nodes.push(Node::new(NodeData::Text {
                    contents: text.to_string(),
                }));
                let id = NodeId(dom.nodes.len() - 1);
                do_append(&mut dom.nodes, id);
                return;
            }
            NodeOrText::AppendNode(id) => id,
        };

        do_append(&mut dom.nodes, new_node);
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Dom;
    type ElemName<'a> = OwnedElemName;

    fn finish(self) -> Dom {
        self.dom.into_inner()
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        self.dom.borrow().document
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> OwnedElemName {
        let dom = self.dom.borrow();
        match &dom.nodes[target.0].data {
            NodeData::Element { name, .. } => OwnedElemName(name.clone()),
            _ => panic!("not an element!"),
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let is_body = &*name.local == "body";
        if is_body {
            self.seen_body.set(true);
        } else if &*name.local == "style" && self.seen_body.get() {
            self.late_style_detected.set(true);
        }

        let template_contents = if flags.template {
            Some(self.alloc(NodeData::Document))
        } else {
            None
        };
        let id = self.alloc(NodeData::Element {
            name,
            attrs,
            template_contents,
        });
        if is_body {
            self.body_id.set(Some(id));
        }
        id
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
        let has_parent = self.dom.borrow().nodes[element.0].parent.is_some();
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
        let mut dom = self.dom.borrow_mut();
        let document = dom.document;
        append(&mut dom.nodes, document, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        match &self.dom.borrow().nodes[target.0].data {
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
        let mut dom = self.dom.borrow_mut();
        let NodeData::Element {
            attrs: existing, ..
        } = &mut dom.nodes[target.0].data
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
        detach(&mut self.dom.borrow_mut().nodes, *target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut dom = self.dom.borrow_mut();
        let mut next_child = dom.nodes[node.0].first_child;
        while let Some(child) = next_child {
            next_child = dom.nodes[child.0].next_sibling;
            append(&mut dom.nodes, *new_parent, child);
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

    /// バイト列を1バイトずつ`feed`しても、一括`parse`と同じDOMになることを
    /// 確認する。タグの途中(`<p`/`>`)・属性の途中・テキストの途中など、
    /// あらゆる位置でチャンクが分割されるケースを網羅する最も厳しい検証。
    #[test]
    fn streaming_parser_byte_by_byte_matches_one_shot_parse() {
        let html = br#"<div class="a"><p>Hello <b>world</b></p></div>"#;

        let mut parser = StreamingParser::new();
        for byte in html {
            parser.feed(std::slice::from_ref(byte));
        }
        let streamed = parser.finish();
        let batched = parse(html);

        let streamed_p = find(&streamed, streamed.document(), "p").expect("p not found");
        let batched_p = find(&batched, batched.document(), "p").expect("p not found");
        assert_eq!(text_of(&streamed, streamed_p), text_of(&batched, batched_p));

        let streamed_div = find(&streamed, streamed.document(), "div").expect("div not found");
        let NodeData::Element { attrs, .. } = &streamed.node(streamed_div).data else {
            panic!("expected element")
        };
        assert_eq!(&*attrs[0].value, "a");
    }

    /// マルチバイトなUTF-8文字("日本語"の各文字は3バイト)がチャンク境界を
    /// またいで分割されても、`Utf8LossyDecoder`のインクリメンタルデコードに
    /// より文字化けせず正しく結合されることを確認する。
    #[test]
    fn streaming_parser_handles_utf8_multibyte_char_split_across_chunks() {
        let html = "<p>日本語のテスト</p>".as_bytes();

        let mut parser = StreamingParser::new();
        for byte in html {
            parser.feed(std::slice::from_ref(byte));
        }
        let dom = parser.finish();

        let p = find(&dom, dom.document(), "p").expect("p not found");
        assert_eq!(text_of(&dom, p), "日本語のテスト");
    }

    /// 複数回に分けて`feed`したテキストは、一括`parse`と同様に隣接テキスト
    /// ノードとして1つに結合されることを確認する(`html::dom`側の結合ロジックが
    /// チャンク分割の影響を受けないことの確認)。
    #[test]
    fn streaming_parser_merges_text_fed_across_multiple_chunks() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<p>Hello");
        parser.feed(b", ");
        parser.feed(b"world!</p>");
        let dom = parser.finish();

        let p = find(&dom, dom.document(), "p").expect("p not found");
        let children: Vec<_> = dom.children(p).collect();
        assert_eq!(
            children.len(),
            1,
            "text fed across multiple chunks should still merge into one node"
        );
        assert_eq!(text_of(&dom, p), "Hello, world!");
    }

    #[test]
    fn has_late_style_tag_is_false_when_style_is_in_head() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<html><head><style>p{color:red}</style></head><body><p>x</p></body></html>");
        assert!(!parser.has_late_style_tag());
    }

    #[test]
    fn has_late_style_tag_is_true_when_style_appears_after_body_starts() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<body><p>x</p><style>p{color:red}</style></body>");
        assert!(parser.has_late_style_tag());
    }

    #[test]
    fn has_late_style_tag_updates_incrementally_across_feed_calls() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<body><p>x</p>");
        assert!(
            !parser.has_late_style_tag(),
            "no <style> tag has appeared yet"
        );
        parser.feed(b"<style>p{color:red}</style>");
        assert!(
            parser.has_late_style_tag(),
            "should detect the <style> tag fed in a later chunk"
        );
    }

    fn tag_of(parser: &StreamingParser, id: NodeId) -> String {
        let dom = parser.dom();
        match &dom.node(id).data {
            NodeData::Element { name, .. } => name.local.to_string(),
            _ => panic!("expected element"),
        }
    }

    #[test]
    fn take_completed_top_level_children_is_empty_before_body_exists() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<html><head><title>t</title></head>");
        assert!(parser.take_completed_top_level_children().is_empty());
    }

    #[test]
    fn take_completed_top_level_children_holds_back_the_last_child() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<body><div>a</div>");
        assert!(
            parser.take_completed_top_level_children().is_empty(),
            "only one top-level child exists so far; it might still grow"
        );
    }

    #[test]
    fn take_completed_top_level_children_yields_once_a_sibling_follows() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<body><div>a</div><p>b</p>");
        let completed = parser.take_completed_top_level_children();
        assert_eq!(completed.len(), 1);
        assert_eq!(tag_of(&parser, completed[0]), "div");

        // まだ2つ目(p)はheldされたまま。
        assert!(parser.take_completed_top_level_children().is_empty());
    }

    #[test]
    fn take_completed_top_level_children_does_not_repeat_already_yielded_nodes() {
        let mut parser = StreamingParser::new();
        parser.feed(b"<body><div>a</div><p>b</p><span>c</span>");
        let first_batch = parser.take_completed_top_level_children();
        assert_eq!(
            first_batch
                .iter()
                .map(|&id| tag_of(&parser, id))
                .collect::<Vec<_>>(),
            vec!["div", "p"]
        );

        parser.feed(b"<footer>d</footer>");
        let second_batch = parser.take_completed_top_level_children();
        assert_eq!(
            second_batch
                .iter()
                .map(|&id| tag_of(&parser, id))
                .collect::<Vec<_>>(),
            vec!["span"],
            "should yield only newly-completed nodes, not repeat earlier ones"
        );
    }
}
