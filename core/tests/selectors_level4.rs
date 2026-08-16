//! Selectors Level 4の`:has()`/`:is()`/`:where()`のE2Eテスト。
//!
//! マッチングは`selectors`クレートの実装をそのまま使うため、ここで固定するのは
//! 「有効になっていること」と「詳細度がカスケードの勝敗に正しく反映されること」。

use std::collections::HashMap;
use std::rc::Rc;

use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::style::{
    compute_styles, parse_stylesheet, user_agent_stylesheet, ComputedStyle,
};

/// `id`属性で要素を探す。
fn find_by_id(dom: &Dom, from: NodeId, id: &str) -> Option<NodeId> {
    if let NodeData::Element { attrs, .. } = &dom.node(from).data {
        if attrs
            .iter()
            .any(|a| &*a.name.local == "id" && &*a.value == id)
        {
            return Some(from);
        }
    }
    dom.children(from).find_map(|c| find_by_id(dom, c, id))
}

fn styles_of(html_src: &str, css: &str) -> (Dom, HashMap<NodeId, Rc<ComputedStyle>>) {
    let dom = html::parse(format!("<body>{html_src}</body>").as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    (dom, styles)
}

/// `#target`の`color`を`#rrggbb`形式で返す。
fn color_of(html_src: &str, css: &str) -> String {
    let (dom, styles) = styles_of(html_src, css);
    let target = find_by_id(&dom, dom.document(), "target").expect("#target がない");
    let c = styles.get(&target).expect("styleがない").color;
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

const RED: &str = "#ff0000";
const BLACK: &str = "#000000";
const BLUE: &str = "#0000ff";

// ===== :has() =====

#[test]
fn has_matches_on_a_descendant() {
    let css = "div:has(img) { color: red }";
    assert_eq!(
        color_of(r#"<div id="target"><p><img src="x"></p></div>"#, css),
        RED
    );
    assert_eq!(
        color_of(r#"<div id="target"><p>text</p></div>"#, css),
        BLACK
    );
}

#[test]
fn has_honours_the_child_combinator() {
    let css = "div:has(> img) { color: red }";
    assert_eq!(
        color_of(r#"<div id="target"><img src="x"></div>"#, css),
        RED
    );
    assert_eq!(
        color_of(r#"<div id="target"><p><img src="x"></p></div>"#, css),
        BLACK,
        "孫は`> img`にマッチしない"
    );
}

#[test]
fn has_honours_sibling_combinators() {
    assert_eq!(
        color_of(
            r#"<h1 id="target">a</h1><p>b</p>"#,
            "h1:has(+ p) { color: red }"
        ),
        RED
    );
    assert_eq!(
        color_of(
            r#"<h1 id="target">a</h1><div>x</div><p>b</p>"#,
            "h1:has(+ p) { color: red }"
        ),
        BLACK,
        "隣接でなければマッチしない"
    );
    assert_eq!(
        color_of(
            r#"<h1 id="target">a</h1><div>x</div><p>b</p>"#,
            "h1:has(~ p) { color: red }"
        ),
        RED
    );
}

#[test]
fn has_can_be_combined_with_not() {
    let css = "div:not(:has(img)) { color: red }";
    assert_eq!(color_of(r#"<div id="target"><p>text</p></div>"#, css), RED);
    assert_eq!(
        color_of(r#"<div id="target"><img src="x"></div>"#, css),
        BLACK
    );
}

#[test]
fn has_takes_the_specificity_of_its_most_specific_argument() {
    // `div:has(#a)` は 1,0,1、`div.c` は 0,1,1。id を含む前者が勝つ。
    let css = "div.c { color: blue } div:has(#a) { color: red }";
    assert_eq!(
        color_of(r#"<div id="target" class="c"><p id="a">x</p></div>"#, css),
        RED
    );

    // 宣言順を入れ替えても結果は変わらない(詳細度で決まっている)。
    let reversed = "div:has(#a) { color: red } div.c { color: blue }";
    assert_eq!(
        color_of(
            r#"<div id="target" class="c"><p id="a">x</p></div>"#,
            reversed
        ),
        RED
    );
}

// ===== :is() / :where() =====

#[test]
fn is_matches_any_of_its_arguments() {
    let css = ":is(h1, h2, h3) { color: red }";
    assert_eq!(color_of(r#"<h2 id="target">x</h2>"#, css), RED);
    assert_eq!(color_of(r#"<h4 id="target">x</h4>"#, css), BLACK);
}

#[test]
fn is_can_be_used_inside_a_complex_selector() {
    let css = "section :is(h1, h2) span { color: red }";
    assert_eq!(
        color_of(
            r#"<section><h2><span id="target">x</span></h2></section>"#,
            css
        ),
        RED
    );
}

#[test]
fn is_takes_the_specificity_of_its_most_specific_argument() {
    // `:is(#target, span)` は 1,0,0。クラスセレクタ(0,1,0)より強い。
    let html = r#"<p id="target" class="c">x</p>"#;
    assert_eq!(
        color_of(html, ".c { color: blue } :is(#target, span) { color: red }"),
        RED
    );
    // 宣言順を入れ替えても結果は変わらない(詳細度で決まっている)。
    assert_eq!(
        color_of(html, ":is(#target, span) { color: red } .c { color: blue }"),
        RED
    );
}

#[test]
fn where_contributes_no_specificity() {
    // `:where(#a)`は0,0,0なので、要素セレクタ(0,0,1)にすら負ける。
    let css = ":where(#a) { color: red } p { color: blue }";
    assert_eq!(color_of(r#"<p id="target">x</p>"#, css), BLUE);

    // 宣言順を入れ替えると、同じ詳細度でないぶん常にpが勝つことを確認する。
    let reversed = "p { color: blue } :where(#a) { color: red }";
    assert_eq!(color_of(r#"<p id="target">x</p>"#, reversed), BLUE);
}

#[test]
fn where_still_matches_even_though_it_adds_no_specificity() {
    assert_eq!(
        color_of(r#"<h2 id="target">x</h2>"#, ":where(h1, h2) { color: red }"),
        RED
    );
}

/// `:is()`/`:where()`の引数リストは寛容(forgiving)なので、未対応のセレクタが
/// 混ざっていてもその項だけが捨てられ、ルール自体は生き残る。
#[test]
fn is_and_where_drop_only_the_unsupported_argument() {
    assert_eq!(
        color_of(
            r#"<h1 id="target">x</h1>"#,
            ":is(h1, ::first-line) { color: red }"
        ),
        RED
    );
}

/// 逆に、通常のセレクタリストは寛容ではない(1つでも壊れているとルールごと
/// 落ちる)。`:is()`との違いを固定しておく。
#[test]
fn a_plain_selector_list_is_not_forgiving() {
    assert_eq!(
        color_of(
            r#"<h1 id="target">x</h1>"#,
            "h1, ::first-line { color: red }"
        ),
        BLACK
    );
}
