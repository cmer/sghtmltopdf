//! `@page`ルール(ページの`size`/`margin`・margin box)のパースと解決。
//!
//! `@page`のブロックは「`size`/`margin`系の通常のプロパティ宣言」と
//! 「16個のmargin box at-rule(`@top-left`等)」が混在する構文で、
//! `stylesheet.rs`の既存パーサーとは別に専用の[`PageRuleParser`]を用いる。
//!
//! `style`クレート内は`layout`クレートに依存しない設計方針
//! ([`crate::layout`]が[`crate::style`]に依存する一方向)のため、ページ
//! サイズの実ピクセル値([`NamedPageSize`]の変換テーブル)はこのファイルに
//! 独立して保持する(`layout::page::PageSize`の同名定数と値を同期させる
//! 必要がある、既知の重複)。

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, QualifiedRuleParser,
    RuleBodyItemParser, RuleBodyParser,
};

use super::properties::{parse_declaration, parse_length, PropertyDeclaration};
use super::stylesheet::DeclarationBlockParser;
use super::values::{ContentPart, LengthPercentageOrAuto, SpecifiedLength};

/// `@page`のページセレクタ(prelude)。名前付きページ(`@page intro`)・
/// `:blank`は非対応(非目標)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSelector {
    All,
    First,
    Left,
    Right,
}

/// margin boxの領域(16個)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarginBoxArea {
    TopLeftCorner,
    TopLeft,
    TopCenter,
    TopRight,
    TopRightCorner,
    LeftTop,
    LeftMiddle,
    LeftBottom,
    RightTop,
    RightMiddle,
    RightBottom,
    BottomLeftCorner,
    BottomLeft,
    BottomCenter,
    BottomRight,
    BottomRightCorner,
}

impl MarginBoxArea {
    /// 16個のmargin box at-rule名との単純な対応表(素直な文字列比較で実装)。
    fn from_at_rule_name(name: &str) -> Option<Self> {
        use MarginBoxArea::*;
        let table: &[(&str, MarginBoxArea)] = &[
            ("top-left-corner", TopLeftCorner),
            ("top-left", TopLeft),
            ("top-center", TopCenter),
            ("top-right", TopRight),
            ("top-right-corner", TopRightCorner),
            ("left-top", LeftTop),
            ("left-middle", LeftMiddle),
            ("left-bottom", LeftBottom),
            ("right-top", RightTop),
            ("right-middle", RightMiddle),
            ("right-bottom", RightBottom),
            ("bottom-left-corner", BottomLeftCorner),
            ("bottom-left", BottomLeft),
            ("bottom-center", BottomCenter),
            ("bottom-right", BottomRight),
            ("bottom-right-corner", BottomRightCorner),
        ];
        table
            .iter()
            .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
            .map(|(_, area)| *area)
    }
}

/// `size`プロパティの名前付きページサイズ。`b4`/`b5`/`ledger`は想定用途
/// (請求書・帳票)での実用性を踏まえ非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedPageSize {
    A4,
    A3,
    A5,
    Letter,
    Legal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageOrientation {
    #[default]
    Portrait,
    Landscape,
}

/// `size`プロパティの指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSizeValue {
    Auto,
    Named(NamedPageSize, PageOrientation),
    Explicit(SpecifiedLength, SpecifiedLength),
}

/// 1つの`@page`ルール(パース結果そのまま、まだ他ルールとの併合前)。
#[derive(Debug, Clone, Default)]
pub struct PageRule {
    pub selector_is_all: bool,
    pub selector: Option<PageSelector>,
    pub size: Option<PageSizeValue>,
    /// `margin`/`margin-top`等(`size`以外の`@page`直下の宣言)。実際に
    /// 意味を持つのはmargin系のみだが、パースは`parse_declaration`を
    /// そのまま再利用するため他のプロパティも構文上は受理される(未使用のまま
    /// 無視される、既知の簡略化)。
    pub margin: Vec<PropertyDeclaration>,
    pub margin_boxes: HashMap<MarginBoxArea, Vec<PropertyDeclaration>>,
}

/// `@page`ブロック内の1アイテム。`RuleBodyItemParser`が要求する
/// 単一の出力型としてまとめる。
enum PageBodyItem {
    Size(PageSizeValue),
    Declarations(Vec<PropertyDeclaration>),
    MarginBox(MarginBoxArea, Vec<PropertyDeclaration>),
}

struct PageRuleParser;

impl<'i> DeclarationParser<'i> for PageRuleParser {
    type Declaration = PageBodyItem;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("size") {
            return Ok(PageBodyItem::Size(parse_page_size(input)?));
        }
        Ok(PageBodyItem::Declarations(parse_declaration(&name, input)?))
    }
}

impl<'i> QualifiedRuleParser<'i> for PageRuleParser {
    type Prelude = ();
    type QualifiedRule = PageBodyItem;
    type Error = ();
}

impl<'i> AtRuleParser<'i> for PageRuleParser {
    type Prelude = MarginBoxArea;
    type AtRule = PageBodyItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        MarginBoxArea::from_at_rule_name(&name).ok_or_else(|| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        let mut declaration_parser = DeclarationBlockParser;
        let declarations = RuleBodyParser::new(input, &mut declaration_parser)
            .filter_map(Result::ok)
            .flatten()
            .collect();
        Ok(PageBodyItem::MarginBox(prelude, declarations))
    }
}

impl<'i> RuleBodyItemParser<'i, PageBodyItem, ()> for PageRuleParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `@page`のprelude(ページセレクタ)をパースする。`:first`/`:left`/`:right`
/// 単体のみ認識(複合・名前付きページは非対応)。
pub(super) fn parse_page_selector<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<PageSelector, ParseError<'i, ()>> {
    if input.is_exhausted() {
        return Ok(PageSelector::All);
    }
    input.expect_colon()?;
    let ident = input.expect_ident()?.clone();
    if ident.eq_ignore_ascii_case("first") {
        Ok(PageSelector::First)
    } else if ident.eq_ignore_ascii_case("left") {
        Ok(PageSelector::Left)
    } else if ident.eq_ignore_ascii_case("right") {
        Ok(PageSelector::Right)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `@page { ... }`のブロック本体をパースする。
pub(super) fn parse_page_rule_block<'i, 't>(
    input: &mut Parser<'i, 't>,
    selector: PageSelector,
) -> PageRule {
    let mut rule_parser = PageRuleParser;
    let mut rule = PageRule {
        selector_is_all: selector == PageSelector::All,
        selector: Some(selector),
        ..PageRule::default()
    };
    for item in RuleBodyParser::new(input, &mut rule_parser).filter_map(Result::ok) {
        match item {
            PageBodyItem::Size(size) => rule.size = Some(size),
            PageBodyItem::Declarations(decls) => rule.margin.extend(decls),
            PageBodyItem::MarginBox(area, decls) => {
                rule.margin_boxes.entry(area).or_default().extend(decls)
            }
        }
    }
    rule
}

/// `size`。`auto` | `<page-size> [portrait | landscape]?` | `<length>{1,2}`。
fn parse_page_size<'i>(input: &mut Parser<'i, '_>) -> Result<PageSizeValue, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(PageSizeValue::Auto);
    }

    let mut named: Option<NamedPageSize> = None;
    let mut orientation: Option<PageOrientation> = None;
    while let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        if named.is_none() {
            let candidate = if ident.eq_ignore_ascii_case("a4") {
                Some(NamedPageSize::A4)
            } else if ident.eq_ignore_ascii_case("a3") {
                Some(NamedPageSize::A3)
            } else if ident.eq_ignore_ascii_case("a5") {
                Some(NamedPageSize::A5)
            } else if ident.eq_ignore_ascii_case("letter") {
                Some(NamedPageSize::Letter)
            } else if ident.eq_ignore_ascii_case("legal") {
                Some(NamedPageSize::Legal)
            } else {
                None
            };
            if let Some(candidate) = candidate {
                named = Some(candidate);
                continue;
            }
        }
        if orientation.is_none() {
            if ident.eq_ignore_ascii_case("portrait") {
                orientation = Some(PageOrientation::Portrait);
                continue;
            }
            if ident.eq_ignore_ascii_case("landscape") {
                orientation = Some(PageOrientation::Landscape);
                continue;
            }
        }
        return Err(input.new_custom_error(()));
    }
    if let Some(named) = named {
        return Ok(PageSizeValue::Named(named, orientation.unwrap_or_default()));
    }
    if orientation.is_some() {
        // `portrait`/`landscape`単体は`<page-size>`と併用が前提の仕様
        // (単体では無効)。
        return Err(input.new_custom_error(()));
    }

    let width = parse_length(input)?;
    let height = input.try_parse(parse_length).unwrap_or(width);
    Ok(PageSizeValue::Explicit(width, height))
}

/// 複数`@page`ルールを併合した最終結果。
#[derive(Debug, Clone, Default)]
pub struct ResolvedPageRule {
    /// 幅・高さ(px)。文書全体で1回だけ解決する(`:first`/`:left`/`:right`の
    /// size宣言は今回反映されない)。
    pub size_px: Option<(f32, f32)>,
    pub margin_top: Option<LengthPercentageOrAuto>,
    pub margin_right: Option<LengthPercentageOrAuto>,
    pub margin_bottom: Option<LengthPercentageOrAuto>,
    pub margin_left: Option<LengthPercentageOrAuto>,
    pub margin_boxes: HashMap<MarginBoxArea, Vec<PropertyDeclaration>>,
}

/// 簡易カスケード。無条件`@page{}`ルールをスタイルシート順に畳み込んだ後、
/// `is_first`/`is_left`に合致する擬似クラス付きルールをmargin boxに
/// ついてのみ畳み込む(`size`/`margin`は無条件ルールのみが有効)。
pub fn resolve_page_rules(rules: &[PageRule], is_first: bool, is_left: bool) -> ResolvedPageRule {
    let mut result = ResolvedPageRule::default();

    for rule in rules.iter().filter(|r| r.selector_is_all) {
        if let Some(size) = rule.size {
            result.size_px = Some(resolve_page_size_px(size));
        }
        apply_margin_declarations(&mut result, &rule.margin);
        merge_margin_boxes(&mut result, &rule.margin_boxes);
    }

    for rule in rules.iter().filter(|r| !r.selector_is_all) {
        let applies = match rule.selector {
            Some(PageSelector::First) => is_first,
            Some(PageSelector::Left) => is_left,
            Some(PageSelector::Right) => !is_left,
            Some(PageSelector::All) | None => false,
        };
        if applies {
            merge_margin_boxes(&mut result, &rule.margin_boxes);
        }
    }

    result
}

/// いずれかのmargin boxの`content`で`counter(pages)`(`counters(pages, ...)`
/// 含む)が使われているかを判定する。`Mode::Streaming`では総ページ数が
/// 原理的に決まらないため、`EngineError::UnsupportedInStreamingMode`を
/// 返すかどうかの判定に使う。
pub fn rules_use_page_count(rules: &[PageRule]) -> bool {
    rules.iter().any(|rule| {
        rule.margin_boxes.values().any(|decls| {
            decls.iter().any(|decl| match decl {
                PropertyDeclaration::Content(Some(parts)) => parts.iter().any(|part| {
                    matches!(
                        part,
                        ContentPart::Counter(name, _) | ContentPart::Counters(name, _, _)
                            if name == "pages"
                    )
                }),
                _ => false,
            })
        })
    })
}

fn apply_margin_declarations(result: &mut ResolvedPageRule, decls: &[PropertyDeclaration]) {
    // `@page`のmargin宣言に`em`/`rem`が使われるのは稀だが、要素という概念が
    // 無いため基準フォントサイズは初期値(16px)固定にする(既知の簡略化)。
    const NOMINAL_FONT_SIZE: f32 = 16.0;
    for decl in decls {
        match decl {
            PropertyDeclaration::MarginTop(v) => {
                result.margin_top = Some(v.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE))
            }
            PropertyDeclaration::MarginRight(v) => {
                result.margin_right = Some(v.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE))
            }
            PropertyDeclaration::MarginBottom(v) => {
                result.margin_bottom = Some(v.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE))
            }
            PropertyDeclaration::MarginLeft(v) => {
                result.margin_left = Some(v.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE))
            }
            _ => {}
        }
    }
}

fn merge_margin_boxes(
    result: &mut ResolvedPageRule,
    margin_boxes: &HashMap<MarginBoxArea, Vec<PropertyDeclaration>>,
) {
    for (area, decls) in margin_boxes {
        result
            .margin_boxes
            .entry(*area)
            .or_default()
            .extend(decls.iter().cloned());
    }
}

/// `layout::page::PageSize`の同名定数と同じ値(96dpi換算)。この関数は
/// `style`クレートが`layout`に依存しない設計方針を保つため、値をここに
/// 複製して持つ(モジュールdoc参照、既知の重複)。
fn resolve_page_size_px(size: PageSizeValue) -> (f32, f32) {
    const NOMINAL_FONT_SIZE: f32 = 16.0;
    let (w, h) = match size {
        PageSizeValue::Auto => return (793.7, 1122.5), // autoはA4相当を既定にする
        PageSizeValue::Named(named, _) => match named {
            NamedPageSize::A4 => (793.7, 1122.5),
            NamedPageSize::A3 => (1122.5, 1587.4),
            NamedPageSize::A5 => (559.4, 793.7),
            NamedPageSize::Letter => (816.0, 1056.0),
            NamedPageSize::Legal => (816.0, 1344.0),
        },
        PageSizeValue::Explicit(w, h) => (
            w.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE).0,
            h.resolve(NOMINAL_FONT_SIZE, NOMINAL_FONT_SIZE).0,
        ),
    };
    if let PageSizeValue::Named(_, PageOrientation::Landscape) = size {
        (h, w)
    } else {
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::parse_stylesheet;

    #[test]
    fn page_rule_parses_size_and_margin() {
        let sheet = parse_stylesheet("@page { size: a4 landscape; margin: 48px; }");
        assert_eq!(sheet.page_rules.len(), 1);
        let rule = &sheet.page_rules[0];
        assert!(rule.selector_is_all);
        assert_eq!(
            rule.size,
            Some(PageSizeValue::Named(
                NamedPageSize::A4,
                PageOrientation::Landscape
            ))
        );
        assert_eq!(
            rule.margin.len(),
            4,
            "margin shorthand should expand to 4 longhands"
        );
    }

    #[test]
    fn page_rule_parses_explicit_two_value_size() {
        let sheet = parse_stylesheet("@page { size: 300px 400px; }");
        let rule = &sheet.page_rules[0];
        assert!(matches!(rule.size, Some(PageSizeValue::Explicit(_, _))));
    }

    #[test]
    fn page_rule_recognizes_pseudo_class_selectors() {
        for (css, expected) in [
            ("@page :first { margin: 0; }", PageSelector::First),
            ("@page :left { margin: 0; }", PageSelector::Left),
            ("@page :right { margin: 0; }", PageSelector::Right),
        ] {
            let sheet = parse_stylesheet(css);
            assert_eq!(sheet.page_rules.len(), 1, "css={css}");
            assert_eq!(sheet.page_rules[0].selector, Some(expected), "css={css}");
            assert!(!sheet.page_rules[0].selector_is_all, "css={css}");
        }
    }

    #[test]
    fn page_rule_rejects_named_pages_and_combined_pseudo_classes() {
        // 名前付きページ・複合擬似クラスは非対応。パースエラーになった@page
        // ルールは無視され、後続のルールには影響しない。
        for css in [
            "@page intro { margin: 0; }",
            "@page :first:left { margin: 0; }",
        ] {
            let sheet = parse_stylesheet(&format!("{css} div {{ color: rgb(1, 2, 3); }}"));
            assert!(sheet.page_rules.is_empty(), "css={css}");
        }
    }

    #[test]
    fn page_rule_parses_margin_box_content() {
        let sheet = parse_stylesheet(
            r#"@page { @top-center { content: "Hello"; } @bottom-right { content: counter(page); } }"#,
        );
        let rule = &sheet.page_rules[0];
        assert!(rule.margin_boxes.contains_key(&MarginBoxArea::TopCenter));
        assert!(rule.margin_boxes.contains_key(&MarginBoxArea::BottomRight));
        assert_eq!(rule.margin_boxes.len(), 2);
    }

    #[test]
    fn page_rule_parses_all_sixteen_margin_box_names() {
        let names = [
            "top-left-corner",
            "top-left",
            "top-center",
            "top-right",
            "top-right-corner",
            "left-top",
            "left-middle",
            "left-bottom",
            "right-top",
            "right-middle",
            "right-bottom",
            "bottom-left-corner",
            "bottom-left",
            "bottom-center",
            "bottom-right",
            "bottom-right-corner",
        ];
        let css = names
            .iter()
            .map(|name| format!("@{name} {{ content: \"x\"; }}"))
            .collect::<String>();
        let sheet = parse_stylesheet(&format!("@page {{ {css} }}"));
        assert_eq!(sheet.page_rules[0].margin_boxes.len(), 16);
    }

    #[test]
    fn resolve_page_rules_uses_only_unconditional_rules_for_size_and_margin() {
        let sheet = parse_stylesheet(
            "@page { size: 300px 400px; margin: 10px; } \
             @page :first { size: 999px 999px; margin: 999px; }",
        );
        let resolved = resolve_page_rules(&sheet.page_rules, true, false);
        // :firstのsize/marginは反映されない。
        assert_eq!(resolved.size_px, Some((300.0, 400.0)));
        assert_eq!(
            resolved.margin_top,
            Some(LengthPercentageOrAuto::LengthPercentage(
                crate::style::LengthPercentage::Length(10.0)
            ))
        );
    }

    #[test]
    fn resolve_page_rules_merges_margin_boxes_by_page_context() {
        let sheet = parse_stylesheet(
            r#"@page { @bottom-center { content: "default"; } }
               @page :first { @bottom-center { content: none; } @top-center { content: "cover"; } }"#,
        );
        let first_page = resolve_page_rules(&sheet.page_rules, true, false);
        let other_page = resolve_page_rules(&sheet.page_rules, false, false);

        // :firstページでは@bottom-centerが上書き(後勝ち)されるため、
        // 無条件ルールのcontent宣言と:first側のcontent宣言の両方が
        // (この順で)入っている(実際にどちらが有効かはcontent解決側の責務)。
        let first_bottom_center = &first_page.margin_boxes[&MarginBoxArea::BottomCenter];
        assert_eq!(first_bottom_center.len(), 2);
        assert!(first_page
            .margin_boxes
            .contains_key(&MarginBoxArea::TopCenter));

        // 他ページには:first専用の@top-centerが無い。
        assert!(!other_page
            .margin_boxes
            .contains_key(&MarginBoxArea::TopCenter));
        assert_eq!(
            other_page.margin_boxes[&MarginBoxArea::BottomCenter].len(),
            1
        );
    }

    #[test]
    fn resolve_page_rules_left_and_right_are_mutually_exclusive_based_on_parity() {
        let sheet = parse_stylesheet(
            r#"@page :left { @top-left { content: "L"; } }
               @page :right { @top-right { content: "R"; } }"#,
        );
        let left_page = resolve_page_rules(&sheet.page_rules, false, true);
        let right_page = resolve_page_rules(&sheet.page_rules, false, false);

        assert!(left_page.margin_boxes.contains_key(&MarginBoxArea::TopLeft));
        assert!(!left_page
            .margin_boxes
            .contains_key(&MarginBoxArea::TopRight));
        assert!(right_page
            .margin_boxes
            .contains_key(&MarginBoxArea::TopRight));
        assert!(!right_page
            .margin_boxes
            .contains_key(&MarginBoxArea::TopLeft));
    }

    #[test]
    fn resolve_page_rules_with_no_page_rules_leaves_everything_unset() {
        let resolved = resolve_page_rules(&[], true, false);
        assert_eq!(resolved.size_px, None);
        assert_eq!(resolved.margin_top, None);
        assert!(resolved.margin_boxes.is_empty());
    }
}
