//! カスケード済み宣言(T3)から、要素ごとの計算スタイルを算出する。
//!
//! プロパティごとに「宣言があればそれを採用(カスケード順で最後に勝ったもの)、
//! なければ継承プロパティは親から継承、そうでなければ初期値」という
//! CSSの計算値算出手順を実装する。

use std::collections::HashMap;

use crate::html::{Dom, NodeData, NodeId};

use super::cascade::matching_declarations;
use super::properties::PropertyDeclaration;
use super::stylesheet::Stylesheet;
use super::values::{Color, Display, Length, LengthPercentage, LengthPercentageOrAuto};

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
    /// 継承プロパティ。
    pub font_size: Length,
    /// 継承プロパティ。
    pub font_family: Vec<String>,
    /// 継承プロパティ。
    pub color: RgbaColor,
    pub background_color: RgbaColor,
}

impl Default for ComputedStyle {
    /// CSSの初期値。`border-style`を扱わないM1では、意図しない既定枠線の描画を
    /// 避けるため`border-width`の初期値は仕様の`medium`ではなく`0`とする。
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
            font_size: Length(16.0),
            font_family: vec!["sans-serif".to_string()],
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
    let mut font_size = None;
    let mut font_family = None;
    let mut color = None;
    let mut background_color = None;

    // カスケード順(優先度昇順)に走査するので、後で見つかったものが自然に勝つ。
    for decl in declarations {
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
            PropertyDeclaration::FontSize(v) => font_size = Some(*v),
            PropertyDeclaration::FontFamily(v) => font_family = Some(v.clone()),
            PropertyDeclaration::Color(v) => color = Some(*v),
            PropertyDeclaration::BackgroundColor(v) => background_color = Some(*v),
        }
    }

    let initial = ComputedStyle::default();
    let inherited_font_size = parent.map_or(initial.font_size, |p| p.font_size);
    let inherited_font_family =
        parent.map_or_else(|| initial.font_family.clone(), |p| p.font_family.clone());
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
        font_size: font_size.unwrap_or(inherited_font_size),
        font_family: font_family.unwrap_or(inherited_font_family),
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
}
