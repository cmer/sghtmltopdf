//! `color-mix()`のE2Eテスト。
//!
//! 期待値はブラウザ(Chrome)の計算値に合わせてある。混色そのものの単体テストは
//! `core/src/style/color_mix.rs`にあり、ここではCSSの構文からカスケードを経て
//! 計算スタイルに落ちるところまでを見る。

use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

fn first_div(dom: &Dom, id: NodeId) -> Option<NodeId> {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == "div" {
            return Some(id);
        }
    }
    dom.children(id).find_map(|c| first_div(dom, c))
}

/// `div`の`color`を`(r, g, b, alpha)`で返す。
fn color_of(value: &str) -> (u8, u8, u8, f32) {
    color_of_with(&format!("div {{ color: {value} }}"))
}

fn color_of_with(css: &str) -> (u8, u8, u8, f32) {
    let dom = html::parse(b"<body><div>x</div></body>");
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let div = first_div(&dom, dom.document()).expect("div がない");
    let c = styles.get(&div).expect("style がない").color;
    (c.red, c.green, c.blue, c.alpha)
}

/// 宣言が落ちたときの色(継承した初期値)。
const DROPPED: (u8, u8, u8, f32) = (0, 0, 0, 1.0);

#[test]
fn mixes_in_srgb() {
    assert_eq!(
        color_of("color-mix(in srgb, red, blue)"),
        (128, 0, 128, 1.0)
    );
}

#[test]
fn a_percentage_shifts_the_balance() {
    assert_eq!(
        color_of("color-mix(in srgb, red 25%, blue)"),
        (64, 0, 191, 1.0)
    );
    // 片方だけ書いた場合、もう片方は残りぶんになる。
    assert_eq!(
        color_of("color-mix(in srgb, red, blue 25%)"),
        (191, 0, 64, 1.0)
    );
}

/// パーセンテージは色の前に書いてもよい。
#[test]
fn a_percentage_may_come_before_the_color() {
    assert_eq!(
        color_of("color-mix(in srgb, 25% red, blue)"),
        (64, 0, 191, 1.0)
    );
}

/// 合計が100%を超える場合は比率だけが意味を持つ。
#[test]
fn weights_over_one_hundred_percent_are_normalised() {
    assert_eq!(
        color_of("color-mix(in srgb, red 50%, blue 150%)"),
        color_of("color-mix(in srgb, red 25%, blue 75%)")
    );
}

/// 合計が100%に満たない場合は、足りないぶんだけ結果が透明になる。
#[test]
fn weights_under_one_hundred_percent_make_the_result_transparent() {
    assert_eq!(
        color_of("color-mix(in srgb, red 25%, blue 25%)"),
        (128, 0, 128, 0.5)
    );
}

#[test]
fn both_weights_at_zero_is_invalid() {
    assert_eq!(color_of("color-mix(in srgb, red 0%, blue 0%)"), DROPPED);
}

/// 知覚的に均等な色空間では、sRGBの算術平均とは違う色になる。
#[test]
fn perceptual_spaces_give_a_different_midpoint() {
    let srgb = color_of("color-mix(in srgb, white, black)");
    let lab = color_of("color-mix(in lab, white, black)");
    assert_eq!(srgb, (128, 128, 128, 1.0));
    assert_eq!(lab, (119, 119, 119, 1.0));
}

#[test]
fn supports_the_polar_spaces() {
    // 赤(0度)と青(240度)の中間は、短い方の弧を通って300度(マゼンタ)。
    assert_eq!(color_of("color-mix(in hsl, red, blue)"), (255, 0, 255, 1.0));
    assert_eq!(
        color_of("color-mix(in hsl longer hue, red, blue)"),
        (0, 255, 0, 1.0)
    );
}

#[test]
fn alpha_is_premultiplied() {
    assert_eq!(
        color_of("color-mix(in srgb, rgba(255, 0, 0, 0.5), blue)"),
        (85, 0, 170, 0.75)
    );
}

#[test]
fn color_mix_can_be_nested() {
    // 内側は紫(128, 0, 128)。それと白の中間。
    assert_eq!(
        color_of("color-mix(in srgb, color-mix(in srgb, red, blue), white)"),
        (192, 128, 192, 1.0)
    );
}

#[test]
fn works_for_other_color_properties() {
    let css = "div { background-color: color-mix(in srgb, red, blue) }";
    let dom = html::parse(b"<body><div>x</div></body>");
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let div = first_div(&dom, dom.document()).unwrap();
    let bg = styles.get(&div).unwrap().background_color;
    assert_eq!((bg.red, bg.green, bg.blue), (128, 0, 128));
}

// ===== 無効な形 =====

/// sRGBより広い色域の空間は、出力先がDeviceRGBでは意味を持たないため非対応。
#[test]
fn wide_gamut_spaces_are_not_supported() {
    for space in ["display-p3", "a98-rgb", "prophoto-rgb", "rec2020"] {
        assert_eq!(
            color_of(&format!("color-mix(in {space}, red, blue)")),
            DROPPED,
            "{space}"
        );
    }
}

#[test]
fn an_unknown_color_space_is_invalid() {
    assert_eq!(color_of("color-mix(in bogus, red, blue)"), DROPPED);
}

/// `currentcolor`はカスケードの後に解決するため、この時点では混ぜられない。
#[test]
fn currentcolor_as_an_operand_is_not_supported() {
    assert_eq!(color_of("color-mix(in srgb, currentcolor, blue)"), DROPPED);
}

#[test]
fn malformed_syntax_is_invalid() {
    for value in [
        "color-mix(red, blue)",                     // `in <space>`がない
        "color-mix(in srgb, red)",                  // 色が1つしかない
        "color-mix(in srgb red, blue)",             // カンマがない
        "color-mix(in srgb, red -10%, blue)",       // 負のパーセンテージ
        "color-mix(in oklch bogus hue, red, blue)", // 未知の色相補間
    ] {
        assert_eq!(color_of(value), DROPPED, "{value}");
    }
}

/// 宣言が落ちても、同じルール内の他の宣言や後続のルールは生き残る。
#[test]
fn an_invalid_color_mix_only_drops_its_own_declaration() {
    let css = "div { color: color-mix(in bogus, red, blue); background-color: red }";
    let dom = html::parse(b"<body><div>x</div></body>");
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let div = first_div(&dom, dom.document()).unwrap();
    let style = styles.get(&div).unwrap();
    assert_eq!(
        (
            style.background_color.red,
            style.background_color.green,
            style.background_color.blue
        ),
        (255, 0, 0)
    );
}
