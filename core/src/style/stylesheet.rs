//! stylesheet全体(ルールの集合)のパース。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use selectors::parser::{ParseRelative, SelectorList};

use super::font_face::{parse_font_face_block, FontFaceRule};
use super::page_rule::{parse_page_rule_block, parse_page_selector, PageRule};
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
    pub page_rules: Vec<PageRule>,
}

/// トップレベルルールの中間表現。通常のスタイルルール・`@font-face`・
/// `@media`・`@page`は`StyleSheetParser`の型システム上、同じ`Prelude`/
/// `Rule`型を共有する必要があるため、この列挙型で束ねる
/// ([`parse_stylesheet`]で仕分ける)。
enum TopLevelRule {
    Style(StyleRule),
    FontFace(FontFaceRule),
    /// `@media`の中身(マッチしなかった場合は空のVec、[0028](
    /// ../../../docs/decisions/0028-paged-media-design.md)決定1)。
    Media(Vec<TopLevelRule>),
    Page(PageRule),
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut rule_parser = TopLevelRuleParser;

    let mut rules = Vec::new();
    let mut font_faces = Vec::new();
    let mut page_rules = Vec::new();
    for result in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
        flatten_top_level_rule(result, &mut rules, &mut font_faces, &mut page_rules);
    }

    Stylesheet {
        rules,
        font_faces,
        page_rules,
    }
}

fn flatten_top_level_rule(
    rule: TopLevelRule,
    rules: &mut Vec<StyleRule>,
    font_faces: &mut Vec<FontFaceRule>,
    page_rules: &mut Vec<PageRule>,
) {
    match rule {
        TopLevelRule::Style(r) => rules.push(r),
        TopLevelRule::FontFace(r) => font_faces.push(r),
        TopLevelRule::Page(r) => page_rules.push(r),
        TopLevelRule::Media(inner) => {
            for r in inner {
                flatten_top_level_rule(r, rules, font_faces, page_rules);
            }
        }
    }
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

/// `@font-face`/`@media`/`@page`を認識する。それ以外の未知at-ruleは
/// デフォルト実装により無視される。
enum TopLevelAtRulePrelude {
    FontFace,
    /// `applies`は[`media_query_list_matches`]による判定結果
    /// ([0028](../../../docs/decisions/0028-paged-media-design.md)決定1)。
    Media {
        applies: bool,
    },
    Page(super::page_rule::PageSelector),
}

impl<'i> AtRuleParser<'i> for TopLevelRuleParser {
    type Prelude = TopLevelAtRulePrelude;
    type AtRule = TopLevelRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("font-face") {
            return Ok(TopLevelAtRulePrelude::FontFace);
        }
        if name.eq_ignore_ascii_case("media") {
            let applies = media_query_list_matches(input)?;
            return Ok(TopLevelAtRulePrelude::Media { applies });
        }
        if name.eq_ignore_ascii_case("page") {
            let selector = parse_page_selector(input)?;
            return Ok(TopLevelAtRulePrelude::Page(selector));
        }
        Err(input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        match prelude {
            TopLevelAtRulePrelude::FontFace => {
                Ok(TopLevelRule::FontFace(parse_font_face_block(input)?))
            }
            TopLevelAtRulePrelude::Page(selector) => {
                Ok(TopLevelRule::Page(parse_page_rule_block(input, selector)))
            }
            // マッチしなかった`@media`ブロックの中身は読み飛ばすだけでよい
            // (`input`は既にこのブロックにスコープされているため、何も
            // 消費せず返しても呼び出し元が正しくブロックの終端まで進める)。
            TopLevelAtRulePrelude::Media { applies: false } => Ok(TopLevelRule::Media(Vec::new())),
            TopLevelAtRulePrelude::Media { applies: true } => {
                let mut rule_parser = TopLevelRuleParser;
                let rules = StyleSheetParser::new(input, &mut rule_parser)
                    .flatten()
                    .collect();
                Ok(TopLevelRule::Media(rules))
            }
        }
    }
}

/// `@media`のprelude(トークン列)から、印刷/PDF出力用の簡略化された
/// メディアタイプ判定を行う。カンマ区切りのクエリリスト(=OR)を、それぞれ
/// 「先頭の`not`/`only`修飾子+メディアタイプ識別子」だけ見て判定する。
/// 特徴クエリ(`(min-width: ...)`等)は一切評価せず読み飛ばす
/// ([0026](../../../docs/decisions/0026-m9-css3-scope-decisions.md)決定2、
/// [0028](../../../docs/decisions/0028-paged-media-design.md)決定1)。
fn media_query_list_matches<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<bool, ParseError<'i, ()>> {
    let mut any_matches = false;
    loop {
        let (matches, has_more) = parse_one_media_query(input)?;
        any_matches = any_matches || matches;
        if !has_more {
            break;
        }
    }
    Ok(any_matches)
}

/// 1つのメディアクエリ(次のコンマまで、またはpreludeの終端まで)を判定し、
/// その分のトークンを消費する。特徴クエリ部分は評価せず読み飛ばす。戻り値は
/// `(このクエリがマッチしたか, まだ後続クエリがあるか)`。
fn parse_one_media_query<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<(bool, bool), ParseError<'i, ()>> {
    let mut negate = false;
    let mut media_type: Option<CowRcStr<'i>> = None;

    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        if ident.eq_ignore_ascii_case("not") {
            negate = true;
            media_type = input.try_parse(|input| input.expect_ident_cloned()).ok();
        } else if ident.eq_ignore_ascii_case("only") {
            media_type = input.try_parse(|input| input.expect_ident_cloned()).ok();
        } else {
            media_type = Some(ident);
        }
    }

    // 残り(`and (min-width: ...)`等)は評価せず、次のコンマ(を消費して
    // 「後続あり」を伝える)またはpreludeの終端まで読み飛ばす。
    let has_more = loop {
        match input.next() {
            Ok(Token::Comma) => break true,
            Ok(_) => continue,
            Err(_) => break false,
        }
    };

    let is_screen = media_type
        .as_deref()
        .map(|ty| ty.eq_ignore_ascii_case("screen"))
        .unwrap_or(false);
    Ok((is_screen == negate, has_more))
}

/// `{ }`内の宣言のみをパースする(ネストしたルールは扱わない)。
/// `@page`のmargin box(`page_rule.rs`)からも再利用する。
pub(super) struct DeclarationBlockParser;

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

/// `Mode::Streaming`では評価できない「後方参照セレクタ」
/// ([0006](../../../docs/decisions/0006-css-non-locality-scope.md)分類3)が
/// 使われていれば、その名前を返す。
///
/// これらは対象要素の親の子リストが完結するまで原理的に判定できないため、
/// ストリーミング処理では**常に非マッチ**になる。黙って結果が変わるのを
/// 避けるため、呼び出し側が警告を出すのに使う([0006]の積み残しへの対応)。
pub fn backward_looking_selectors(sheet: &Stylesheet) -> Vec<String> {
    use cssparser::ToCss as _;

    const NAMES: &[&str] = &[
        ":nth-last-child",
        ":nth-last-of-type",
        ":last-child",
        ":last-of-type",
        ":only-child",
        ":only-of-type",
        ":empty",
    ];

    let mut found: Vec<String> = Vec::new();
    for rule in &sheet.rules {
        for selector in rule.selectors.slice() {
            let text = selector.to_css_string();
            for name in NAMES {
                if text.contains(name) && !found.iter().any(|f| f == name) {
                    found.push((*name).to_string());
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {

    #[test]
    fn backward_looking_selectors_are_detected() {
        let sheet = parse_stylesheet(
            "li:last-child { color: red } p:nth-last-child(2) { color: blue } div:empty { color: green }",
        );
        let found = backward_looking_selectors(&sheet);
        assert!(found.contains(&":last-child".to_string()), "got: {found:?}");
        assert!(
            found.contains(&":nth-last-child".to_string()),
            "got: {found:?}"
        );
        assert!(found.contains(&":empty".to_string()), "got: {found:?}");
    }

    #[test]
    fn ordinary_selectors_are_not_reported_as_backward_looking() {
        let sheet = parse_stylesheet(
            "li:first-child { color: red } p + p { color: blue } a:hover { color: green }",
        );
        assert!(backward_looking_selectors(&sheet).is_empty());
    }
    use super::*;

    #[test]
    fn media_print_and_all_rules_are_applied() {
        for query in ["print", "all", "print, screen", "not screen"] {
            let sheet = parse_stylesheet(&format!(
                "@media {query} {{ div {{ color: rgb(1, 2, 3); }} }}"
            ));
            assert_eq!(
                sheet.rules.len(),
                1,
                "@media {query} should apply its rules"
            );
        }
    }

    #[test]
    fn media_screen_only_rules_are_ignored() {
        let sheet = parse_stylesheet("@media screen { div { color: rgb(1, 2, 3); } }");
        assert!(
            sheet.rules.is_empty(),
            "@media screen should not apply its rules"
        );
    }

    #[test]
    fn media_with_only_a_feature_query_defaults_to_matching_all() {
        // 型を省略した特徴クエリ単体(`(min-width: 600px)`)は仕様上
        // `all and (min-width: 600px)`と同義。特徴クエリ自体は評価しない
        // ([0026]決定2)ため、常にマッチする。
        let sheet = parse_stylesheet("@media (min-width: 600px) { div { color: rgb(1, 2, 3); } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn media_rules_and_subsequent_rules_are_both_parsed() {
        let sheet = parse_stylesheet(
            "@media print { div { color: rgb(1, 2, 3); } } p { color: rgb(4, 5, 6); }",
        );
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn nested_media_rules_are_flattened() {
        let sheet =
            parse_stylesheet("@media print { @media all { div { color: rgb(1, 2, 3); } } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn font_face_inside_a_matching_media_rule_is_still_recognized() {
        let sheet = parse_stylesheet(
            r#"@media print { @font-face { font-family: "Test"; src: url("test.ttf"); } }"#,
        );
        assert_eq!(sheet.font_faces.len(), 1);
    }

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
        //
        // M7(T73-76)で`@import`を実際にフェッチ・展開する機能を追加したが、
        // それは`style::extract::extract_author_stylesheet`が`parse_stylesheet`
        // を呼ぶ「前」にCSSテキストを展開する形で実装されている
        // (`style::import::resolve_imports`、[0016](../../../docs/decisions/0016-at-import-resolution-design.md)参照)。
        // `parse_stylesheet`自体は今も`@import`を知らないままであり、この
        // テストが検証する「安全に無視される」という挙動は変わらず正しい。
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
        // T63: `url()`参照を含む値を持つが、本実装が対応していないプロパティが
        // あっても、そのプロパティ宣言だけが無視され、同じルール内の他の宣言・
        // 後続のルールは正常にパースされることを確認する。
        //
        // 元々は`background-image: url(...)`をこの「未対応プロパティ」の例に
        // 使っていたが、M7(T80)で`background-image`自体を実装したため、
        // 今も非対応の`border-image`に差し替えた(`background-position`等の
        // 他のbackground-*系プロパティと同じく、マイルストーン8/9のCSS3対応へ
        // 先送り)。
        let sheet =
            parse_stylesheet(r#"div { border-image: url("border.png") 30; color: rgb(1, 2, 3); }"#);
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(
            sheet.rules[0].declarations.len(),
            1,
            "the unrecognized border-image declaration should be skipped, \
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
