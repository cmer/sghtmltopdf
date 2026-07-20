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
    fn parse_stylesheet_ignores_at_import_and_keeps_subsequent_rules() {
        // T63(M6): `@import`は`@font-face`以外のat-ruleとして
        // `TopLevelRuleParser::parse_prelude`が拒否し、cssparserの
        // `StyleSheetParser`のエラー回復で読み飛ばされる想定。フェッチした
        // 外部CSSに`@import`が含まれていても、それ以降の通常ルールの
        // パースが継続されることを確認する(追加実装なしで安全、という
        // T59/T63の前提調査を実際に検証する)。
        let sheet = parse_stylesheet(
            r#"@import url("other.css"); p { color: rgb(1, 2, 3); } div { color: rgb(4, 5, 6); }"#,
        );
        assert_eq!(
            sheet.rules.len(),
            2,
            "both rules after the ignored @import should still be parsed"
        );
    }

    #[test]
    fn parse_stylesheet_ignores_unrecognized_properties_with_url_values() {
        // T63: `background-image: url(...)`のような、本実装が対応して
        // いないプロパティ(`url()`参照を含む値)があっても、そのプロパティ
        // 宣言だけが無視され、同じルール内の他の宣言・後続のルールは
        // 正常にパースされることを確認する。
        let sheet =
            parse_stylesheet(r#"div { background-image: url("bg.png"); color: rgb(1, 2, 3); }"#);
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(
            sheet.rules[0].declarations.len(),
            1,
            "the unrecognized background-image declaration should be skipped, \
             leaving only the color declaration"
        );
        assert!(matches!(
            sheet.rules[0].declarations[0],
            PropertyDeclaration::Color(_)
        ));
    }

    #[test]
    fn parse_inline_style_handles_empty_string() {
        assert!(parse_inline_style("").is_empty());
    }
}
