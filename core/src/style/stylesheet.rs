//! stylesheet全体(ルールの集合)のパース。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use selectors::parser::{ParseRelative, SelectorList};

use super::font_face::{parse_font_face_block, FontFaceRule};
use super::properties::{parse_declaration, PropertyDeclaration};
use super::selector_impl::{SelectorParser, SgSelectorImpl};

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: SelectorList<SgSelectorImpl>,
    pub declarations: Vec<PropertyDeclaration>,
}

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    pub font_faces: Vec<FontFaceRule>,
}

/// トップレベルルールの中間表現。通常のスタイルルールと`@font-face`は
/// `StyleSheetParser`の型システム上、同じ`Prelude`/`Rule`型を共有する必要が
/// あるため、この列挙型で束ねる([`parse_stylesheet`]で仕分ける)。
enum TopLevelRule {
    Style(StyleRule),
    FontFace(FontFaceRule),
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut rule_parser = TopLevelRuleParser;

    let mut rules = Vec::new();
    let mut font_faces = Vec::new();
    for result in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
        match result {
            TopLevelRule::Style(rule) => rules.push(rule),
            TopLevelRule::FontFace(rule) => font_faces.push(rule),
        }
    }

    Stylesheet { rules, font_faces }
}

/// `style="..."`属性の値のような、セレクタを伴わない宣言リストをパースする。
pub fn parse_inline_style(css: &str) -> Vec<PropertyDeclaration> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut declaration_parser = DeclarationBlockParser;

    RuleBodyParser::new(&mut parser, &mut declaration_parser)
        .filter_map(Result::ok)
        .flatten()
        .collect()
}

/// stylesheet直下のルール(セレクタ+宣言ブロック)をパースする。
/// M1では`@media`等のat-ruleは非対応(デフォルト実装により無視される)。
struct TopLevelRuleParser;

impl<'i> QualifiedRuleParser<'i> for TopLevelRuleParser {
    type Prelude = SelectorList<SgSelectorImpl>;
    type QualifiedRule = TopLevelRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        SelectorList::parse(&SelectorParser, input, ParseRelative::No)
            .map_err(|_| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        let mut declaration_parser = DeclarationBlockParser;
        let declarations = RuleBodyParser::new(input, &mut declaration_parser)
            .filter_map(Result::ok)
            .flatten()
            .collect();
        Ok(TopLevelRule::Style(StyleRule {
            selectors,
            declarations,
        }))
    }
}

/// `@font-face`のみを認識する。それ以外のat-rule(`@media`等)は
/// デフォルト実装により無視される。
impl<'i> AtRuleParser<'i> for TopLevelRuleParser {
    type Prelude = ();
    type AtRule = TopLevelRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("font-face") {
            Ok(())
        } else {
            Err(input.new_custom_error(()))
        }
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        Ok(TopLevelRule::FontFace(parse_font_face_block(input)?))
    }
}

/// `{ }`内の宣言のみをパースする(ネストしたルールは扱わない)。
struct DeclarationBlockParser;

impl<'i> DeclarationParser<'i> for DeclarationBlockParser {
    type Declaration = Vec<PropertyDeclaration>;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        parse_declaration(&name, input)
    }
}

impl<'i> QualifiedRuleParser<'i> for DeclarationBlockParser {
    type Prelude = ();
    type QualifiedRule = Vec<PropertyDeclaration>;
    type Error = ();
}

impl<'i> AtRuleParser<'i> for DeclarationBlockParser {
    type Prelude = ();
    type AtRule = Vec<PropertyDeclaration>;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Vec<PropertyDeclaration>, ()> for DeclarationBlockParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inline_style_parses_bare_declarations() {
        let decls = parse_inline_style("color: rgb(1, 2, 3); font-size: 14px");
        assert_eq!(decls.len(), 2);
        assert!(matches!(decls[0], PropertyDeclaration::Color(_)));
        assert!(matches!(decls[1], PropertyDeclaration::FontSize(_)));
    }

    #[test]
    fn parse_inline_style_ignores_unknown_properties() {
        let decls = parse_inline_style("not-a-real-property: 5px; color: rgb(1, 2, 3)");
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0], PropertyDeclaration::Color(_)));
    }

    #[test]
    fn parse_inline_style_handles_empty_string() {
        assert!(parse_inline_style("").is_empty());
    }
}
