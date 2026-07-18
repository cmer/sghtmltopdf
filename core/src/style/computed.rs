//! カスケード済み宣言(T3)から、要素ごとの計算スタイルを算出する。
//!
//! プロパティごとに「宣言があればそれを採用(カスケード順で最後に勝ったもの)、
//! なければ継承プロパティは親から継承、そうでなければ初期値」という
//! CSSの計算値算出手順を実装する。

use std::collections::HashMap;

use crate::html::{Dom, NodeData, NodeId};

use super::cascade::matching_declarations;
use super::properties::PropertyDeclaration;
use super::stylesheet::{parse_inline_style, Stylesheet};
use super::values::{
    BorderStyle, Color, Display, FontStyle, FontWeight, Length, LengthPercentage,
    LengthPercentageOrAuto,
};

/// `color`/`background-color`の計算値。パース時と異なり`currentcolor`は解決済み。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbaColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub width: LengthPercentageOrAuto,
    pub height: LengthPercentageOrAuto,
    pub margin_top: LengthPercentageOrAuto,
    pub margin_right: LengthPercentageOrAuto,
    pub margin_bottom: LengthPercentageOrAuto,
    pub margin_left: LengthPercentageOrAuto,
    pub padding_top: LengthPercentage,
    pub padding_right: LengthPercentage,
    pub padding_bottom: LengthPercentage,
    pub padding_left: LengthPercentage,
    pub border_top_width: Length,
    pub border_right_width: Length,
    pub border_bottom_width: Length,
    pub border_left_width: Length,
    /// 初期値は`currentcolor`(仕様通り)。宣言がなければこの要素自身の
    /// 計算済み`color`を使う(`resolve_color`で解決)。
    pub border_top_color: RgbaColor,
    pub border_right_color: RgbaColor,
    pub border_bottom_color: RgbaColor,
    pub border_left_color: RgbaColor,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub border_top_left_radius: Length,
    pub border_top_right_radius: Length,
    pub border_bottom_right_radius: Length,
    pub border_bottom_left_radius: Length,
    /// 継承プロパティ。
    pub font_size: Length,
    /// 継承プロパティ。
    pub font_family: Vec<String>,
    /// 継承プロパティ。
    pub font_weight: FontWeight,
    /// 継承プロパティ。
    pub font_style: FontStyle,
    /// 継承プロパティ。
    pub color: RgbaColor,
    pub background_color: RgbaColor,
}

impl Default for ComputedStyle {
    /// CSSの初期値。`border-width`の初期値は仕様上`medium`(実装依存の太さ、
    /// 概ね3px相当)だが、意図しない既定枠線の描画を避けるためここでは`0`とする
    /// (どのみち`border-style`の初期値`none`により、幅があっても描画はされない)。
    fn default() -> Self {
        let zero_lp = LengthPercentage::Length(0.0);
        Self {
            display: Display::Inline,
            width: LengthPercentageOrAuto::Auto,
            height: LengthPercentageOrAuto::Auto,
            margin_top: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_right: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_bottom: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_left: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            padding_top: zero_lp,
            padding_right: zero_lp,
            padding_bottom: zero_lp,
            padding_left: zero_lp,
            border_top_width: Length(0.0),
            border_right_width: Length(0.0),
            border_bottom_width: Length(0.0),
            border_left_width: Length(0.0),
            // currentcolorの初期解決先(このデフォルト値自体が親を持たない場合の
            // 基準になる)。実際の解決は`resolve_color`が行う。
            border_top_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_right_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_bottom_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_left_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,
            border_top_left_radius: Length(0.0),
            border_top_right_radius: Length(0.0),
            border_bottom_right_radius: Length(0.0),
            border_bottom_left_radius: Length(0.0),
            font_size: Length(16.0),
            font_family: vec!["sans-serif".to_string()],
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            background_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0.0,
            },
        }
    }
}

/// DOM全体の計算スタイルを求める。要素以外のノード(テキスト等)には
/// ボックスに関するプロパティは意味を持たないため、単純に親の計算スタイルを
/// (= 継承プロパティも含めてそのまま)引き継ぐ。
pub fn compute_styles(
    dom: &Dom,
    ua: &Stylesheet,
    author: &Stylesheet,
) -> HashMap<NodeId, ComputedStyle> {
    let mut styles = HashMap::new();
    compute_recursive(dom, dom.document(), None, ua, author, &mut styles);
    styles
}

fn compute_recursive(
    dom: &Dom,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    ua: &Stylesheet,
    author: &Stylesheet,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    let style = match &dom.node(node).data {
        NodeData::Element { .. } => compute_element_style(dom, node, parent_style, ua, author),
        _ => parent_style.cloned().unwrap_or_default(),
    };

    for child in dom.children(node) {
        compute_recursive(dom, child, Some(&style), ua, author, out);
    }

    out.insert(node, style);
}

fn compute_element_style(
    dom: &Dom,
    element: NodeId,
    parent: Option<&ComputedStyle>,
    ua: &Stylesheet,
    author: &Stylesheet,
) -> ComputedStyle {
    let declarations = matching_declarations(dom, element, ua, author);
    let inline_declarations = inline_style_declarations(dom, element);

    let mut display = None;
    let mut width = None;
    let mut height = None;
    let mut margin_top = None;
    let mut margin_right = None;
    let mut margin_bottom = None;
    let mut margin_left = None;
    let mut padding_top = None;
    let mut padding_right = None;
    let mut padding_bottom = None;
    let mut padding_left = None;
    let mut border_top_width = None;
    let mut border_right_width = None;
    let mut border_bottom_width = None;
    let mut border_left_width = None;
    let mut border_top_color = None;
    let mut border_right_color = None;
    let mut border_bottom_color = None;
    let mut border_left_color = None;
    let mut border_top_style = None;
    let mut border_right_style = None;
    let mut border_bottom_style = None;
    let mut border_left_style = None;
    let mut border_top_left_radius = None;
    let mut border_top_right_radius = None;
    let mut border_bottom_right_radius = None;
    let mut border_bottom_left_radius = None;
    let mut font_size = None;
    let mut font_family = None;
    let mut font_weight = None;
    let mut font_style = None;
    let mut color = None;
    let mut background_color = None;

    // カスケード順(優先度昇順)に走査するので、後で見つかったものが自然に勝つ。
    // インラインstyle属性はセレクタベースのどの宣言よりも優先度が高いため、最後に置く。
    for decl in declarations.into_iter().chain(inline_declarations.iter()) {
        match decl {
            PropertyDeclaration::Display(v) => display = Some(*v),
            PropertyDeclaration::Width(v) => width = Some(*v),
            PropertyDeclaration::Height(v) => height = Some(*v),
            PropertyDeclaration::MarginTop(v) => margin_top = Some(*v),
            PropertyDeclaration::MarginRight(v) => margin_right = Some(*v),
            PropertyDeclaration::MarginBottom(v) => margin_bottom = Some(*v),
            PropertyDeclaration::MarginLeft(v) => margin_left = Some(*v),
            PropertyDeclaration::PaddingTop(v) => padding_top = Some(*v),
            PropertyDeclaration::PaddingRight(v) => padding_right = Some(*v),
            PropertyDeclaration::PaddingBottom(v) => padding_bottom = Some(*v),
            PropertyDeclaration::PaddingLeft(v) => padding_left = Some(*v),
            PropertyDeclaration::BorderTopWidth(v) => border_top_width = Some(*v),
            PropertyDeclaration::BorderRightWidth(v) => border_right_width = Some(*v),
            PropertyDeclaration::BorderBottomWidth(v) => border_bottom_width = Some(*v),
            PropertyDeclaration::BorderLeftWidth(v) => border_left_width = Some(*v),
            PropertyDeclaration::BorderTopColor(v) => border_top_color = Some(*v),
            PropertyDeclaration::BorderRightColor(v) => border_right_color = Some(*v),
            PropertyDeclaration::BorderBottomColor(v) => border_bottom_color = Some(*v),
            PropertyDeclaration::BorderLeftColor(v) => border_left_color = Some(*v),
            PropertyDeclaration::BorderTopStyle(v) => border_top_style = Some(*v),
            PropertyDeclaration::BorderRightStyle(v) => border_right_style = Some(*v),
            PropertyDeclaration::BorderBottomStyle(v) => border_bottom_style = Some(*v),
            PropertyDeclaration::BorderLeftStyle(v) => border_left_style = Some(*v),
            PropertyDeclaration::BorderTopLeftRadius(v) => border_top_left_radius = Some(*v),
            PropertyDeclaration::BorderTopRightRadius(v) => border_top_right_radius = Some(*v),
            PropertyDeclaration::BorderBottomRightRadius(v) => {
                border_bottom_right_radius = Some(*v)
            }
            PropertyDeclaration::BorderBottomLeftRadius(v) => border_bottom_left_radius = Some(*v),
            PropertyDeclaration::FontSize(v) => font_size = Some(*v),
            PropertyDeclaration::FontFamily(v) => font_family = Some(v.clone()),
            PropertyDeclaration::FontWeight(v) => font_weight = Some(*v),
            PropertyDeclaration::FontStyle(v) => font_style = Some(*v),
            PropertyDeclaration::Color(v) => color = Some(*v),
            PropertyDeclaration::BackgroundColor(v) => background_color = Some(*v),
        }
    }

    let initial = ComputedStyle::default();
    let inherited_font_size = parent.map_or(initial.font_size, |p| p.font_size);
    let inherited_font_family =
        parent.map_or_else(|| initial.font_family.clone(), |p| p.font_family.clone());
    let inherited_font_weight = parent.map_or(initial.font_weight, |p| p.font_weight);
    let inherited_font_style = parent.map_or(initial.font_style, |p| p.font_style);
    let inherited_color = parent.map_or(initial.color, |p| p.color);

    let resolved_color = resolve_color(color, inherited_color);
    let resolved_background_color = match background_color {
        Some(Color::Rgba {
            red,
            green,
            blue,
            alpha,
        }) => RgbaColor {
            red,
            green,
            blue,
            alpha,
        },
        // `background-color: currentcolor`は、この要素自身の計算済みcolorを使う。
        Some(Color::CurrentColor) => resolved_color,
        None => initial.background_color,
    };
    // `border-color`の初期値は仕様上`currentcolor`なので、未指定時も
    // (`currentcolor`指定時と同様に)この要素自身の計算済みcolorへ解決する。
    let resolved_border_top_color = resolve_color(border_top_color, resolved_color);
    let resolved_border_right_color = resolve_color(border_right_color, resolved_color);
    let resolved_border_bottom_color = resolve_color(border_bottom_color, resolved_color);
    let resolved_border_left_color = resolve_color(border_left_color, resolved_color);

    ComputedStyle {
        display: display.unwrap_or(initial.display),
        width: width.unwrap_or(initial.width),
        height: height.unwrap_or(initial.height),
        margin_top: margin_top.unwrap_or(initial.margin_top),
        margin_right: margin_right.unwrap_or(initial.margin_right),
        margin_bottom: margin_bottom.unwrap_or(initial.margin_bottom),
        margin_left: margin_left.unwrap_or(initial.margin_left),
        padding_top: padding_top.unwrap_or(initial.padding_top),
        padding_right: padding_right.unwrap_or(initial.padding_right),
        padding_bottom: padding_bottom.unwrap_or(initial.padding_bottom),
        padding_left: padding_left.unwrap_or(initial.padding_left),
        border_top_width: border_top_width.unwrap_or(initial.border_top_width),
        border_right_width: border_right_width.unwrap_or(initial.border_right_width),
        border_bottom_width: border_bottom_width.unwrap_or(initial.border_bottom_width),
        border_left_width: border_left_width.unwrap_or(initial.border_left_width),
        border_top_color: resolved_border_top_color,
        border_right_color: resolved_border_right_color,
        border_bottom_color: resolved_border_bottom_color,
        border_left_color: resolved_border_left_color,
        border_top_style: border_top_style.unwrap_or(initial.border_top_style),
        border_right_style: border_right_style.unwrap_or(initial.border_right_style),
        border_bottom_style: border_bottom_style.unwrap_or(initial.border_bottom_style),
        border_left_style: border_left_style.unwrap_or(initial.border_left_style),
        border_top_left_radius: border_top_left_radius.unwrap_or(initial.border_top_left_radius),
        border_top_right_radius: border_top_right_radius.unwrap_or(initial.border_top_right_radius),
        border_bottom_right_radius: border_bottom_right_radius
            .unwrap_or(initial.border_bottom_right_radius),
        border_bottom_left_radius: border_bottom_left_radius
            .unwrap_or(initial.border_bottom_left_radius),
        font_size: font_size.unwrap_or(inherited_font_size),
        font_family: font_family.unwrap_or(inherited_font_family),
        font_weight: font_weight.unwrap_or(inherited_font_weight),
        font_style: font_style.unwrap_or(inherited_font_style),
        color: resolved_color,
        background_color: resolved_background_color,
    }
}

/// `color`は継承プロパティなので、指定がない場合・`currentcolor`が指定された場合
/// (仕様上は循環するため、継承値をそのまま使う)のいずれも親の計算値を使う。
fn resolve_color(declared: Option<Color>, inherited: RgbaColor) -> RgbaColor {
    match declared {
        Some(Color::Rgba {
            red,
            green,
            blue,
            alpha,
        }) => RgbaColor {
            red,
            green,
            blue,
            alpha,
        },
        Some(Color::CurrentColor) | None => inherited,
    }
}

/// 要素の`style="..."`属性をパースする(属性がなければ空)。
fn inline_style_declarations(dom: &Dom, element: NodeId) -> Vec<PropertyDeclaration> {
    let NodeData::Element { attrs, .. } = &dom.node(element).data else {
        return Vec::new();
    };
    attrs
        .iter()
        .find(|attr| &*attr.name.local == "style")
        .map(|attr| parse_inline_style(&attr.value))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use crate::style::parse_stylesheet;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    #[test]
    fn inherits_color_and_font_family_through_multiple_levels() {
        let dom = html::parse(br#"<div><section><p>text</p></section></div>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { color: rgb(9, 8, 7); font-family: Georgia; }");

        let styles = compute_styles(&dom, &ua, &author);
        let p_style = &styles[&p];

        assert_eq!(
            p_style.color,
            RgbaColor {
                red: 9,
                green: 8,
                blue: 7,
                alpha: 1.0
            }
        );
        assert_eq!(p_style.font_family, vec!["Georgia".to_string()]);
    }

    #[test]
    fn reassigning_inherited_property_stops_old_value_propagation() {
        let dom = html::parse(br#"<div><section><p>text</p></section></div>"#);
        let section = find(&dom, dom.document(), "section").expect("section not found");
        let p = find(&dom, section, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("div { color: rgb(9, 8, 7); } section { color: rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&section].color,
            RgbaColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 1.0
            }
        );
        assert_eq!(
            styles[&p].color,
            RgbaColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn background_color_is_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-color: rgb(5, 5, 5); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 5,
                green: 5,
                blue: 5,
                alpha: 1.0
            }
        );
        assert_eq!(
            styles[&p].background_color,
            ComputedStyle::default().background_color
        );
    }

    #[test]
    fn current_color_background_resolves_to_own_computed_color() {
        let dom = html::parse(br#"<div>text</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("div { color: rgb(4, 5, 6); background-color: currentcolor; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 4,
                green: 5,
                blue: 6,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn root_without_declarations_gets_initial_values() {
        let dom = html::parse(br#"<div>text</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        let default = ComputedStyle::default();
        assert_eq!(styles[&div].color, default.color);
        assert_eq!(styles[&div].font_size, default.font_size);
        assert_eq!(styles[&div].font_family, default.font_family);
    }

    #[test]
    fn non_element_nodes_inherit_parent_style_directly() {
        let dom = html::parse(br#"<p>hello</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let text = dom.children(p).next().expect("text node not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("p { color: rgb(7, 7, 7); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&text], styles[&p]);
    }

    #[test]
    fn inline_style_overrides_stylesheet_rules_regardless_of_specificity() {
        // #idセレクタは通常どのクラス/type選択子よりも詳細度が高いが、
        // インラインstyleはそれよりもさらに優先されるはず。
        let dom = html::parse(br#"<div id="x" style="color: rgb(9, 9, 9);">t</div>"#);
        let p = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("#x { color: rgb(1, 1, 1); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&p].color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn inline_style_applies_when_there_is_no_matching_rule() {
        let dom = html::parse(br#"<div style="background-color: rgb(4, 5, 6);">t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 4,
                green: 5,
                blue: 6,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn font_weight_and_style_are_inherited_but_overridable() {
        let dom = html::parse(br#"<p><b>bold <i>bold-italic</i></b></p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let b = find(&dom, p, "b").expect("b not found");
        let i = find(&dom, b, "i").expect("i not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("b { font-weight: bold; } i { font-style: italic; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&p].font_weight, super::FontWeight::Normal);
        assert_eq!(styles[&b].font_weight, super::FontWeight::Bold);
        assert_eq!(styles[&b].font_style, super::FontStyle::Normal);
        // <i>は<b>からfont-weight: boldを継承しつつ、自身のfont-style: italicを追加する。
        assert_eq!(styles[&i].font_weight, super::FontWeight::Bold);
        assert_eq!(styles[&i].font_style, super::FontStyle::Italic);
    }

    #[test]
    fn numeric_font_weight_is_thresholded_to_bold_or_normal() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let light = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { font-weight: 400; }"),
        );
        assert_eq!(light[&p].font_weight, super::FontWeight::Normal);

        let heavy = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { font-weight: 700; }"),
        );
        assert_eq!(heavy[&p].font_weight, super::FontWeight::Bold);
    }

    #[test]
    fn elements_without_style_attribute_are_unaffected() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div], ComputedStyle::default());
    }

    #[test]
    fn border_shorthand_sets_width_style_and_color_on_all_sides() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border: 2px dashed rgb(10, 20, 30); }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_top_width.0, 2.0);
        assert_eq!(style.border_right_width.0, 2.0);
        assert_eq!(style.border_bottom_width.0, 2.0);
        assert_eq!(style.border_left_width.0, 2.0);
        assert_eq!(style.border_top_style, super::BorderStyle::Dashed);
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 10,
                green: 20,
                blue: 30,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn border_color_defaults_to_currentcolor_when_unspecified() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { color: rgb(9, 9, 9); border: 1px solid; }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            },
            "border-color should follow currentcolor when not explicitly set"
        );
    }

    #[test]
    fn border_color_and_border_style_shorthands_expand_per_side() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            "div { border-style: solid dotted; border-color: rgb(1,1,1) rgb(2,2,2); }",
        );

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_top_style, super::BorderStyle::Solid);
        assert_eq!(style.border_right_style, super::BorderStyle::Dotted);
        assert_eq!(style.border_bottom_style, super::BorderStyle::Solid);
        assert_eq!(style.border_left_style, super::BorderStyle::Dotted);
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 1,
                green: 1,
                blue: 1,
                alpha: 1.0
            }
        );
        assert_eq!(
            style.border_right_color,
            RgbaColor {
                red: 2,
                green: 2,
                blue: 2,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn border_is_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border: 3px solid rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&p].border_top_style, super::BorderStyle::None);
        assert_eq!(styles[&p].border_top_width.0, 0.0);
    }
}
