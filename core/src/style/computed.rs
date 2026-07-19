//! カスケード済み宣言(T3)から、要素ごとの計算スタイルを算出する。
//!
//! プロパティごとに「宣言があればそれを採用(カスケード順で最後に勝ったもの)、
//! なければ継承プロパティは親から継承、そうでなければ初期値」という
//! CSSの計算値算出手順を実装する。

use std::cell::Cell;
use std::collections::HashMap;

use crate::html::{Dom, NodeData, NodeId};

use super::cascade::{matching_declarations, matching_pseudo_content};
use super::properties::PropertyDeclaration;
use super::selector_impl::PseudoElement;
use super::stylesheet::{parse_inline_style, Stylesheet};
use super::values::{
    BorderStyle, BreakBetween, BreakInside, Color, Display, FontStyle, FontWeight, Length,
    LengthPercentage, LengthPercentageOrAuto, SpecifiedLength, SpecifiedLengthPercentage,
    SpecifiedLengthPercentageOrAuto, TextDecorationLine,
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
    /// `text-decoration-line`。仕様上は非継承プロパティだが、代わりに祖先の
    /// 装飾線が子孫のボックスへ「伝播」する特殊規則を持つ。この伝播を
    /// 別途実装する代わりに、継承プロパティとして扱うことで
    /// (`<u>bold <b>text</b></u>`のような)一般的なネストケースで見た目を一致させる
    /// 簡略実装。子孫側で明示的に上書きされれば通常の継承同様そちらが勝つ。
    pub text_decoration_line: TextDecorationLine,
    /// `::before { content: "..." }`の生成コンテンツ。この要素自身のスタイルを
    /// そのまま流用して描画する(擬似要素専用の計算スタイルは持たない簡略実装)。
    pub pseudo_before_content: Option<String>,
    /// `::after { content: "..." }`の生成コンテンツ。
    pub pseudo_after_content: Option<String>,
    /// CSS Fragmentation。非継承プロパティ(仕様通り)。
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    pub break_inside: BreakInside,
    /// ページ末尾に残せる最小行数。非継承プロパティ、初期値2(仕様通り)。
    pub orphans: u32,
    /// ページ先頭に送れる最小行数。非継承プロパティ、初期値2(仕様通り)。
    pub widows: u32,
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
            text_decoration_line: TextDecorationLine::default(),
            pseudo_before_content: None,
            pseudo_after_content: None,
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            orphans: 2,
            widows: 2,
        }
    }
}

/// DOM全体の計算スタイルを求める。要素以外のノード(テキスト等)には
/// ボックスに関するプロパティは意味を持たないため、単純に親の計算スタイルを
/// (= 継承プロパティも含めてそのまま)引き継ぐ。
///
/// `rem`の基準となるルート要素(`<html>`)のフォントサイズは、木を辿りながら
/// 最初に見つかった要素で確定する。それより前の(まだ確定していない)時点では
/// 初期値`16px`を仮の基準として使うが、ルート要素自身が`rem`単位で
/// 自分自身のfont-sizeを指定するような通常あり得ない記述を除けば、
/// ルート要素の子孫は必ずルート確定後に処理されるため実用上問題ない。
pub fn compute_styles(
    dom: &Dom,
    ua: &Stylesheet,
    author: &Stylesheet,
) -> HashMap<NodeId, ComputedStyle> {
    let mut styles = HashMap::new();
    let ctx = StyleContext {
        ua,
        author,
        root_font_size: Cell::new(ComputedStyle::default().font_size.0),
    };
    compute_recursive(dom, dom.document(), None, false, &ctx, &mut styles);
    styles
}

/// `compute_recursive`/`compute_element_style`の再帰全体で共有する、
/// 木を辿る間変化しない(または`Cell`経由で一方向にのみ更新される)値。
/// 引数の数を抑えるための単純なまとめ役。
struct StyleContext<'a> {
    ua: &'a Stylesheet,
    author: &'a Stylesheet,
    /// `rem`の基準となるルート要素(`<html>`)の計算済みフォントサイズ。
    /// 木を辿りながら最初に見つかった要素で確定する。
    root_font_size: Cell<f32>,
}

fn compute_recursive(
    dom: &Dom,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    is_root_candidate: bool,
    ctx: &StyleContext<'_>,
    out: &mut HashMap<NodeId, ComputedStyle>,
) {
    let style = match &dom.node(node).data {
        NodeData::Element { .. } => {
            let style = compute_element_style(
                dom,
                node,
                parent_style,
                ctx.root_font_size.get(),
                ctx.ua,
                ctx.author,
            );
            // ドキュメント直下の最初の要素(通常は<html>)がルート要素。
            if is_root_candidate {
                ctx.root_font_size.set(style.font_size.0);
            }
            style
        }
        _ => parent_style.cloned().unwrap_or_default(),
    };

    // `node`がドキュメントノードであれば、その直下の子(通常は<html>)がルート要素候補。
    let children_are_root_candidates = node == dom.document();
    for child in dom.children(node) {
        compute_recursive(
            dom,
            child,
            Some(&style),
            children_are_root_candidates,
            ctx,
            out,
        );
    }

    out.insert(node, style);
}

fn compute_element_style(
    dom: &Dom,
    element: NodeId,
    parent: Option<&ComputedStyle>,
    root_font_size: f32,
    ua: &Stylesheet,
    author: &Stylesheet,
) -> ComputedStyle {
    let declarations = matching_declarations(dom, element, ua, author);
    let inline_declarations = inline_style_declarations(dom, element);
    let attribute_sugar_declarations = data_page_break_declarations(dom, element);
    let pseudo_before_content =
        matching_pseudo_content(dom, element, PseudoElement::Before, ua, author);
    let pseudo_after_content =
        matching_pseudo_content(dom, element, PseudoElement::After, ua, author);

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
    let mut text_decoration_line = None;
    let mut break_before = None;
    let mut break_after = None;
    let mut break_inside = None;
    let mut orphans = None;
    let mut widows = None;

    // カスケード順(優先度昇順)に走査するので、後で見つかったものが自然に勝つ。
    // `data-page-break`属性糖衣は「スタイルシートで個別に上書きできる既定のヒント」
    // という位置づけのため最も弱く先頭に置く。インラインstyle属性はセレクタベースの
    // どの宣言よりも優先度が高いため、最後に置く。
    for decl in attribute_sugar_declarations
        .iter()
        .chain(declarations)
        .chain(inline_declarations.iter())
    {
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
            PropertyDeclaration::TextDecorationLine(v) => text_decoration_line = Some(*v),
            // `content`は`::before`/`::after`専用で、通常の要素では効果を持たない
            // (`matching_pseudo_content`が別途、擬似要素向けのマッチングを行う)。
            PropertyDeclaration::Content(_) => {}
            PropertyDeclaration::BreakBefore(v) => break_before = Some(*v),
            PropertyDeclaration::BreakAfter(v) => break_after = Some(*v),
            PropertyDeclaration::BreakInside(v) => break_inside = Some(*v),
            PropertyDeclaration::Orphans(v) => orphans = Some(*v),
            PropertyDeclaration::Widows(v) => widows = Some(*v),
        }
    }

    let initial = ComputedStyle::default();
    let inherited_font_size = parent.map_or(initial.font_size, |p| p.font_size);
    let inherited_font_family =
        parent.map_or_else(|| initial.font_family.clone(), |p| p.font_family.clone());
    let inherited_font_weight = parent.map_or(initial.font_weight, |p| p.font_weight);
    let inherited_font_style = parent.map_or(initial.font_style, |p| p.font_style);
    let inherited_color = parent.map_or(initial.color, |p| p.color);
    let inherited_text_decoration_line =
        parent.map_or(initial.text_decoration_line, |p| p.text_decoration_line);

    // font-sizeは他の長さ系プロパティより先に解決する。`em`の基準は仕様上
    // 「親要素の計算済みfont-size」(自分自身の値ではない、循環を避けるため)。
    let resolved_font_size = font_size
        .map(|specified| specified.resolve(inherited_font_size.0, root_font_size))
        .unwrap_or(inherited_font_size);
    // font-size以外の長さ系プロパティの`em`基準は、この要素自身の(今解決した)font-size。
    let own_font_size = resolved_font_size.0;
    let resolve_lp_or_auto = |v: Option<SpecifiedLengthPercentageOrAuto>,
                              initial: LengthPercentageOrAuto| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolve_lp = |v: Option<SpecifiedLengthPercentage>, initial: LengthPercentage| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolve_len = |v: Option<SpecifiedLength>, initial: Length| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };

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
        width: resolve_lp_or_auto(width, initial.width),
        height: resolve_lp_or_auto(height, initial.height),
        margin_top: resolve_lp_or_auto(margin_top, initial.margin_top),
        margin_right: resolve_lp_or_auto(margin_right, initial.margin_right),
        margin_bottom: resolve_lp_or_auto(margin_bottom, initial.margin_bottom),
        margin_left: resolve_lp_or_auto(margin_left, initial.margin_left),
        padding_top: resolve_lp(padding_top, initial.padding_top),
        padding_right: resolve_lp(padding_right, initial.padding_right),
        padding_bottom: resolve_lp(padding_bottom, initial.padding_bottom),
        padding_left: resolve_lp(padding_left, initial.padding_left),
        border_top_width: resolve_len(border_top_width, initial.border_top_width),
        border_right_width: resolve_len(border_right_width, initial.border_right_width),
        border_bottom_width: resolve_len(border_bottom_width, initial.border_bottom_width),
        border_left_width: resolve_len(border_left_width, initial.border_left_width),
        border_top_color: resolved_border_top_color,
        border_right_color: resolved_border_right_color,
        border_bottom_color: resolved_border_bottom_color,
        border_left_color: resolved_border_left_color,
        border_top_style: border_top_style.unwrap_or(initial.border_top_style),
        border_right_style: border_right_style.unwrap_or(initial.border_right_style),
        border_bottom_style: border_bottom_style.unwrap_or(initial.border_bottom_style),
        border_left_style: border_left_style.unwrap_or(initial.border_left_style),
        border_top_left_radius: resolve_len(border_top_left_radius, initial.border_top_left_radius),
        border_top_right_radius: resolve_len(
            border_top_right_radius,
            initial.border_top_right_radius,
        ),
        border_bottom_right_radius: resolve_len(
            border_bottom_right_radius,
            initial.border_bottom_right_radius,
        ),
        border_bottom_left_radius: resolve_len(
            border_bottom_left_radius,
            initial.border_bottom_left_radius,
        ),
        font_size: resolved_font_size,
        font_family: font_family.unwrap_or(inherited_font_family),
        font_weight: font_weight.unwrap_or(inherited_font_weight),
        font_style: font_style.unwrap_or(inherited_font_style),
        color: resolved_color,
        background_color: resolved_background_color,
        text_decoration_line: text_decoration_line.unwrap_or(inherited_text_decoration_line),
        pseudo_before_content,
        pseudo_after_content,
        break_before: break_before.unwrap_or(initial.break_before),
        break_after: break_after.unwrap_or(initial.break_after),
        break_inside: break_inside.unwrap_or(initial.break_inside),
        orphans: orphans.unwrap_or(initial.orphans),
        widows: widows.unwrap_or(initial.widows),
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

/// `data-page-break="before|after|avoid"`属性の糖衣API。対応する`break-before`/
/// `break-after`/`break-inside: avoid`宣言へ変換する(値は大文字小文字を区別しない)。
/// 認識できない値は無視する(通常のCSSの不正値と同様、宣言なしとして扱う)。
fn data_page_break_declarations(dom: &Dom, element: NodeId) -> Vec<PropertyDeclaration> {
    let NodeData::Element { attrs, .. } = &dom.node(element).data else {
        return Vec::new();
    };
    let Some(attr) = attrs
        .iter()
        .find(|attr| &*attr.name.local == "data-page-break")
    else {
        return Vec::new();
    };
    match attr.value.trim().to_ascii_lowercase().as_str() {
        "before" => vec![PropertyDeclaration::BreakBefore(BreakBetween::Always)],
        "after" => vec![PropertyDeclaration::BreakAfter(BreakBetween::Always)],
        "avoid" => vec![PropertyDeclaration::BreakInside(BreakInside::Avoid)],
        _ => Vec::new(),
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
    fn hsl_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // 純粋な赤(hue=0, saturation=100%, lightness=50%) = rgb(255, 0, 0)。
        let author = parse_stylesheet("div { color: hsl(0deg 100% 50%); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].color,
            RgbaColor {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn hwb_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // 白100% -> 完全な白 rgb(255, 255, 255)。
        let author = parse_stylesheet("div { color: hwb(0deg 100% 0%); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].color,
            RgbaColor {
                red: 255,
                green: 255,
                blue: 255,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn hsl_color_function_with_alpha_is_preserved() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-color: hsl(0deg 0% 0% / 50%); }");

        let styles = compute_styles(&dom, &ua, &author);
        let bg = styles[&div].background_color;
        assert_eq!((bg.red, bg.green, bg.blue), (0, 0, 0));
        assert!((bg.alpha - 0.5).abs() < 0.01);
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
    fn text_decoration_line_parses_underline_and_line_through() {
        let dom = html::parse(br#"<p>a</p>"#);

        let underline = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration: underline; }"),
        );
        let p = find(&dom, dom.document(), "p").expect("p not found");
        assert!(underline[&p].text_decoration_line.underline);
        assert!(!underline[&p].text_decoration_line.line_through);

        let line_through = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration-line: line-through; }"),
        );
        assert!(line_through[&p].text_decoration_line.line_through);
        assert!(!line_through[&p].text_decoration_line.underline);

        let both = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration: underline line-through; }"),
        );
        assert!(both[&p].text_decoration_line.underline);
        assert!(both[&p].text_decoration_line.line_through);
    }

    #[test]
    fn text_decoration_line_propagates_to_descendants_like_font_weight() {
        // 仕様上は非継承だが、祖先の装飾線が子孫へ伝播する特殊規則の代わりに
        // このリポジトリでは継承として扱う簡略実装(computed.rsのコメント参照)。
        let dom = html::parse(br#"<u>bold <b>text</b></u>"#);
        let u = find(&dom, dom.document(), "u").expect("u not found");
        let b = find(&dom, u, "b").expect("b not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("u { text-decoration: underline; }"),
        );
        assert!(styles[&u].text_decoration_line.underline);
        assert!(styles[&b].text_decoration_line.underline);
    }

    #[test]
    fn ua_stylesheet_gives_u_and_s_their_default_text_decoration() {
        use super::super::ua::user_agent_stylesheet;

        let dom = html::parse(br#"<p><u>underlined</u> <s>struck</s></p>"#);
        let u = find(&dom, dom.document(), "u").expect("u not found");
        let s = find(&dom, dom.document(), "s").expect("s not found");

        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());
        assert!(styles[&u].text_decoration_line.underline);
        assert!(styles[&s].text_decoration_line.line_through);
    }

    #[test]
    fn text_decoration_none_overrides_inherited_underline() {
        let dom = html::parse(br#"<u><span class="plain">text</span></u>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("u { text-decoration: underline; } .plain { text-decoration: none; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert!(!styles[&span].text_decoration_line.underline);
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
    fn em_font_size_resolves_against_parent_font_size() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        // div: 20px、p: divの1.5倍 = 30px。
        let author = parse_stylesheet("div { font-size: 20px; } p { font-size: 1.5em; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].font_size.0, 20.0);
        assert_eq!(styles[&p].font_size.0, 30.0);
    }

    #[test]
    fn em_length_on_non_font_size_property_uses_own_font_size() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // font-sizeが先に20pxへ解決され、border-widthの2emはそれを基準にする = 40px。
        let author = parse_stylesheet("div { font-size: 20px; border: 2em solid black; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].border_top_width.0, 40.0);
    }

    #[test]
    fn rem_length_resolves_against_root_element_font_size_regardless_of_nesting() {
        let dom = html::parse(br#"<html><body><div><p>text</p></div></body></html>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let ua = Stylesheet::default();
        // ルート(<html>)のfont-sizeを10pxにし、ネストしたpのmargin: 2remが
        // 親(div/body)のfont-sizeに影響されず常に20pxになることを確認する。
        let author = parse_stylesheet(
            "html { font-size: 10px; } div { font-size: 30px; } p { margin: 2rem; }",
        );

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&p].margin_top,
            LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(20.0))
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

    #[test]
    fn break_before_and_break_after_default_to_auto() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].break_after, BreakBetween::Auto);
        assert_eq!(styles[&p].break_inside, BreakInside::Auto);
    }

    #[test]
    fn break_before_and_break_after_parse_avoid_and_always() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: avoid; break-after: always; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Avoid);
        assert_eq!(styles[&p].break_after, BreakBetween::Always);
    }

    #[test]
    fn break_before_page_keyword_is_treated_as_always() {
        // 単一ページサイズしか扱わないため、`page`は`always`と同じ効果として扱う。
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: page; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Always);
    }

    #[test]
    fn break_inside_parses_avoid() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-inside: avoid; }"),
        );
        assert_eq!(styles[&p].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn legacy_page_break_properties_are_aliases_for_break_properties() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "p { page-break-before: always; page-break-after: avoid; \
                 page-break-inside: avoid; }",
            ),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Always);
        assert_eq!(styles[&p].break_after, BreakBetween::Avoid);
        assert_eq!(styles[&p].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn orphans_and_widows_default_to_two_and_can_be_overridden() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&p].orphans, 2);
        assert_eq!(defaults[&p].widows, 2);

        let overridden = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { orphans: 3; widows: 4; }"),
        );
        assert_eq!(overridden[&p].orphans, 3);
        assert_eq!(overridden[&p].widows, 4);
    }

    #[test]
    fn orphans_rejects_non_positive_values() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        // 無効な値は宣言ごと無視され、初期値のままになる。
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { orphans: 0; }"),
        );
        assert_eq!(styles[&p].orphans, 2);
    }

    #[test]
    fn break_properties_are_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { break-before: always; orphans: 5; }"),
        );
        assert_eq!(styles[&div].break_before, BreakBetween::Always);
        assert_eq!(styles[&div].orphans, 5);
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].orphans, 2);
    }

    #[test]
    fn data_page_break_attribute_maps_to_break_properties() {
        let dom = html::parse(
            br#"<div><p id="a" data-page-break="before">a</p>
                <p id="b" data-page-break="after">b</p>
                <p id="c" data-page-break="avoid">c</p></div>"#,
        );
        let a = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&a].break_before, BreakBetween::Always);

        let mut ps = Vec::new();
        fn find_all(dom: &Dom, id: NodeId, out: &mut Vec<NodeId>) {
            if let NodeData::Element { name, .. } = &dom.node(id).data {
                if &*name.local == "p" {
                    out.push(id);
                }
            }
            for child in dom.children(id) {
                find_all(dom, child, out);
            }
        }
        find_all(&dom, dom.document(), &mut ps);
        assert_eq!(styles[&ps[1]].break_after, BreakBetween::Always);
        assert_eq!(styles[&ps[2]].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn data_page_break_ignores_unrecognized_values() {
        let dom = html::parse(br#"<p data-page-break="sideways">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].break_after, BreakBetween::Auto);
        assert_eq!(styles[&p].break_inside, BreakInside::Auto);
    }

    #[test]
    fn stylesheet_rule_overrides_data_page_break_attribute() {
        // 属性糖衣は「スタイルシートで個別に上書きできる既定のヒント」という
        // 位置づけなので、通常のCSSルールの方が優先される。
        let dom = html::parse(br#"<p data-page-break="before">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: auto; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
    }

    #[test]
    fn inline_style_overrides_data_page_break_attribute() {
        let dom = html::parse(br#"<p data-page-break="before" style="break-before: auto;">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
    }

    #[test]
    fn before_and_after_pseudo_content_resolve_from_matching_rules() {
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet(r#".badge::before { content: "["; } .badge::after { content: "]"; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content.as_deref(), Some("["));
        assert_eq!(styles[&span].pseudo_after_content.as_deref(), Some("]"));
    }

    #[test]
    fn pseudo_content_is_none_without_a_matching_before_after_rule() {
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(".badge { color: rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
        assert_eq!(styles[&span].pseudo_after_content, None);
    }

    #[test]
    fn explicit_content_none_wins_over_an_earlier_lower_specificity_rule() {
        let dom = html::parse(br#"<span id="x" class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        // クラスセレクタで文字列を指定していても、詳細度の高い#idセレクタが
        // 後から`content: none`にすれば生成ボックスは無くなるはず。
        let author =
            parse_stylesheet(r#".badge::before { content: "x"; } #x::before { content: none; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
    }

    #[test]
    fn pseudo_content_ignores_declarations_on_the_real_element() {
        // `::before`/`::after`を伴わない通常のセレクタでの`content`宣言は無効。
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(r#".badge { content: "x"; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
        assert_eq!(styles[&span].pseudo_after_content, None);
    }
}
