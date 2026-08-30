//! CSS Nesting(ネストしたスタイルルール)のE2Eテスト(waka/sghtmltopdf#25)。
//!
//! `&`の置き換えとネストの解決は`selectors`クレートの実装を使うため、ここで
//! 固定するのは「ネストしたルールが捨てられずにカスケードへ届くこと」と
//! 「詳細度・ソース順が仕様どおりカスケードの勝敗に反映されること」。

use std::collections::HashMap;
use std::rc::Rc;

use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::style::{
    compute_styles, parse_stylesheet, user_agent_stylesheet, ComputedStyle, LengthPercentage,
    LengthPercentageOrAuto,
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

fn style_of(html_src: &str, css: &str) -> Rc<ComputedStyle> {
    let (dom, styles) = styles_of(html_src, css);
    let target = find_by_id(&dom, dom.document(), "target").expect("#target がない");
    Rc::clone(styles.get(&target).expect("styleがない"))
}

/// `#target`の`color`を`#rrggbb`形式で返す。
fn color_of(html_src: &str, css: &str) -> String {
    let c = style_of(html_src, css).color;
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

/// `#target`の`margin-left`をpxで返す。
fn margin_left_of(html_src: &str, css: &str) -> f32 {
    match style_of(html_src, css).margin_left {
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => px,
        other => panic!("margin-left が長さではない: {other:?}"),
    }
}

const RED: &str = "#ff0000";
const BLACK: &str = "#000000";
const BLUE: &str = "#0000ff";

const NESTED_PROBE: &str = r#"<div class="wrap"><div class="probe" id="target">X</div></div>"#;

// ===== issue #25 の再現ケース =====

#[test]
fn flat_control_rule_applies() {
    let css = ".wrap .probe { margin-left: 90px }";
    assert_eq!(margin_left_of(NESTED_PROBE, css), 90.0);
}

#[test]
fn nested_rule_with_explicit_parent_selector_applies() {
    let css = ".wrap { & .probe { margin-left: 90px } }";
    assert_eq!(margin_left_of(NESTED_PROBE, css), 90.0);
}

#[test]
fn nested_rule_with_implicit_parent_selector_applies() {
    let css = ".wrap { .probe { margin-left: 90px } }";
    assert_eq!(margin_left_of(NESTED_PROBE, css), 90.0);
}

#[test]
fn nested_compound_parent_selector_applies() {
    let css = ".wrap { &.probe { margin-left: 90px } }";
    assert_eq!(
        margin_left_of(r#"<div class="wrap probe" id="target">X</div>"#, css),
        90.0
    );
    assert_eq!(
        margin_left_of(NESTED_PROBE, css),
        0.0,
        "`&.probe`は`.wrap.probe`であって`.wrap .probe`ではない"
    );
}

#[test]
fn nested_rule_with_leading_combinator_applies() {
    // `margin-left`は継承しないので、孫が親から値を受け継ぐことはない。
    let css = ".list { > li { margin-left: 90px } }";
    assert_eq!(
        margin_left_of(r#"<ul class="list"><li id="target">a</li></ul>"#, css),
        90.0
    );
    assert_eq!(
        margin_left_of(
            r#"<ul class="list"><li><ul><li id="target">a</li></ul></li></ul>"#,
            css
        ),
        0.0,
        "孫は`> li`にマッチしない"
    );
}

#[test]
fn nested_type_selector_is_not_mistaken_for_a_declaration() {
    // `p {`は宣言(`p:`)と同じくidentで始まるので、宣言として読めなければ
    // ルールとして読み直す必要がある。
    let css = ".wrap { p { color: red } }";
    assert_eq!(
        color_of(r#"<div class="wrap"><p id="target">a</p></div>"#, css),
        RED
    );
}

#[test]
fn nested_pseudo_class_selector_is_not_mistaken_for_a_declaration() {
    // `a:link { }`は`a: link { }`という宣言にも見える。
    let css = ".wrap { a:link { color: red } }";
    assert_eq!(
        color_of(
            r##"<div class="wrap"><a href="#" id="target">a</a></div>"##,
            css
        ),
        RED
    );
}

#[test]
fn nesting_can_be_deeper_than_one_level() {
    let css = ".a { .b { .c { color: red } } }";
    assert_eq!(
        color_of(
            r#"<div class="a"><div class="b"><div class="c" id="target">x</div></div></div>"#,
            css
        ),
        RED
    );
    assert_eq!(
        color_of(
            r#"<div class="a"><div class="c" id="target">x</div></div>"#,
            css
        ),
        BLACK,
        "`.b`を飛ばした要素にはマッチしない"
    );
}

#[test]
fn nested_rule_under_a_selector_list_applies_to_every_parent() {
    let css = ".a, .b { .c { color: red } }";
    assert_eq!(
        color_of(
            r#"<div class="b"><div class="c" id="target">x</div></div>"#,
            css
        ),
        RED
    );
}

#[test]
fn nested_rule_does_not_match_outside_its_parent() {
    let css = ".wrap { .probe { color: red } }";
    assert_eq!(
        color_of(r#"<div class="probe" id="target">X</div>"#, css),
        BLACK
    );
}

// ===== 親ルールの宣言との共存 =====

#[test]
fn declarations_before_a_nested_rule_still_apply_to_the_parent() {
    let css = ".wrap { color: red; .probe { margin-left: 90px } }";
    assert_eq!(
        color_of(r#"<div class="wrap" id="target">X</div>"#, css),
        RED
    );
}

#[test]
fn declarations_after_a_nested_rule_still_apply_to_the_parent() {
    let css = ".wrap { .probe { margin-left: 90px } color: red }";
    assert_eq!(
        color_of(r#"<div class="wrap" id="target">X</div>"#, css),
        RED
    );
}

#[test]
fn declarations_after_a_nested_rule_cascade_after_it() {
    // 仕様(CSSNestedDeclarations)では、ネストしたルールの後ろにある宣言は
    // そのルールより後ろの位置でカスケードに参加する。先頭へ巻き上げない。
    let css = ".probe { & { color: red } color: blue }";
    assert_eq!(
        color_of(r#"<div class="probe" id="target">X</div>"#, css),
        BLUE
    );
}

// ===== 詳細度 =====

#[test]
fn nested_selector_takes_the_parent_specificity() {
    // `#wrap { & .probe }` = (1,1,0) は後続の`.wrap .probe` = (0,2,0)に勝つ。
    let css = "#wrap { & .probe { color: red } } .wrap .probe { color: blue }";
    assert_eq!(
        color_of(
            r#"<div id="wrap" class="wrap"><div class="probe" id="target">X</div></div>"#,
            css
        ),
        RED
    );
}

#[test]
fn equal_specificity_falls_back_to_source_order() {
    let css = ".wrap { & .probe { color: red } } .wrap .probe { color: blue }";
    assert_eq!(color_of(NESTED_PROBE, css), BLUE);
}

// ===== エラー回復 =====

#[test]
fn an_invalid_nested_rule_does_not_take_its_siblings_with_it() {
    // `::first-line`は非対応なのでそのネストしたルールだけが捨てられ、
    // 後続の宣言と兄弟のネストしたルールは生き残る。
    let css = ".wrap { .probe::first-line { color: blue } color: red; .probe { color: red } }";
    assert_eq!(
        color_of(r#"<div class="wrap" id="target">X</div>"#, css),
        RED
    );
    assert_eq!(color_of(NESTED_PROBE, css), RED);
}

// ===== トップレベルの`&` =====

#[test]
fn a_top_level_parent_selector_acts_as_scope() {
    // 置き換える親が無い`&`は仕様どおり`:scope`、スタイルシートではルート要素。
    // `color`は継承するので、`html`に効いたことを子孫で観測する。
    let css = "& { color: red }";
    assert_eq!(color_of(r#"<div id="target">X</div>"#, css), RED);
    assert_eq!(
        margin_left_of(r#"<div id="target">X</div>"#, "& { margin-left: 90px }"),
        0.0,
        "ルート以外の要素にはマッチしない"
    );
}
