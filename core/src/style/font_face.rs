//! `@font-face`ルールのパース。
//!
//! `font-family`/`src`は必須ディスクリプタとして扱い、どちらか一方でも
//! 欠けていればルール全体を無効として捨てる(CSSの仕様通り)。

use cssparser::{
    match_ignore_ascii_case, AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, UnicodeRange,
};

use super::properties::{parse_family_name, parse_font_style, parse_font_weight};
use super::values::{FontStyle, FontWeight};

#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    pub family: String,
    /// `src`に列挙された`url(...)`/`local(...)`を、記述順のまま保持する
    /// (実際の解決・優先順位判断は呼び出し側の責務)。
    pub src: Vec<FontFaceSource>,
    pub weight: FontWeight,
    pub style: FontStyle,
    /// `unicode-range`。空`Vec`は「未指定」を意味し、全域(U+0-10FFFF)を
    /// 暗黙にカバーするものとして扱う(呼び出し側の責務)。
    pub unicode_range: Vec<UnicodeRange>,
}

/// `src`ディスクリプタの1エントリ。
#[derive(Debug, Clone, PartialEq)]
pub enum FontFaceSource {
    /// `url(...)`。相対パスの解決は呼び出し側の責務(HTMLファイルの
    /// ディレクトリ基準)。
    Url(String),
    /// `local(...)`。システムフォントのフルネームまたはPostScript名。
    Local(String),
}

/// `@font-face { ... }`の宣言ブロックをパースし、`font-family`/`src`が
/// どちらも得られた場合のみ[`FontFaceRule`]を返す。
pub(super) fn parse_font_face_block<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FontFaceRule, ParseError<'i, ()>> {
    let mut parser = FontFaceDeclarationParser;
    let descriptors: Vec<FontFaceDescriptor> = RuleBodyParser::new(input, &mut parser)
        .filter_map(Result::ok)
        .collect();

    let mut family = None;
    let mut src = None;
    let mut weight = FontWeight::default();
    let mut style = FontStyle::default();
    let mut unicode_range = Vec::new();

    for descriptor in descriptors {
        match descriptor {
            FontFaceDescriptor::Family(v) => family = Some(v),
            FontFaceDescriptor::Src(v) => src = Some(v),
            FontFaceDescriptor::Weight(v) => weight = v,
            FontFaceDescriptor::Style(v) => style = v,
            FontFaceDescriptor::UnicodeRange(v) => unicode_range = v,
        }
    }

    let family = family.ok_or_else(|| input.new_custom_error(()))?;
    let src = src.ok_or_else(|| input.new_custom_error(()))?;
    Ok(FontFaceRule {
        family,
        src,
        weight,
        style,
        unicode_range,
    })
}

enum FontFaceDescriptor {
    Family(String),
    Src(Vec<FontFaceSource>),
    Weight(FontWeight),
    Style(FontStyle),
    UnicodeRange(Vec<UnicodeRange>),
}

struct FontFaceDeclarationParser;

impl<'i> DeclarationParser<'i> for FontFaceDeclarationParser {
    type Declaration = FontFaceDescriptor;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        match_ignore_ascii_case! { &name,
            "font-family" => Ok(FontFaceDescriptor::Family(parse_family_name(input)?)),
            "src" => Ok(FontFaceDescriptor::Src(parse_font_face_src(input)?)),
            "font-weight" => Ok(FontFaceDescriptor::Weight(parse_font_weight(input)?)),
            "font-style" => Ok(FontFaceDescriptor::Style(parse_font_style(input)?)),
            "unicode-range" => Ok(FontFaceDescriptor::UnicodeRange(parse_unicode_range(input)?)),
            _ => Err(input.new_custom_error(())),
        }
    }
}

impl<'i> QualifiedRuleParser<'i> for FontFaceDeclarationParser {
    type Prelude = ();
    type QualifiedRule = FontFaceDescriptor;
    type Error = ();
}

impl<'i> AtRuleParser<'i> for FontFaceDeclarationParser {
    type Prelude = ();
    type AtRule = FontFaceDescriptor;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, FontFaceDescriptor, ()> for FontFaceDeclarationParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `src`ディスクリプタ: `url(...)`/`local(...)`のカンマ区切りリスト。各エントリの
/// 後ろに続く`format(...)`/`tech(...)`ヒントは中身を検証せず読み飛ばす(対応
/// フォーマットかどうかの判定はロード時の実際のパース結果に委ねるため)。
fn parse_font_face_src<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<FontFaceSource>, ParseError<'i, ()>> {
    let mut sources = Vec::new();
    input.parse_comma_separated(|input| {
        if let Ok(url) = input.try_parse(|input| input.expect_url_or_string()) {
            sources.push(FontFaceSource::Url(url.as_ref().to_string()));
        } else {
            input
                .expect_function_matching("local")
                .map_err(|_| input.new_custom_error(()))?;
            let name = input.parse_nested_block(|input| {
                let Ok(name) = input.expect_ident_or_string() else {
                    return Err(input.new_custom_error(()));
                };
                Ok(name.as_ref().to_string())
            })?;
            sources.push(FontFaceSource::Local(name));
        }

        while let Ok(name) = input.try_parse(|input| input.expect_function().cloned()) {
            if !name.eq_ignore_ascii_case("format") && !name.eq_ignore_ascii_case("tech") {
                return Err(input.new_custom_error(()));
            }
            input.parse_nested_block(|input| {
                while input.next().is_ok() {}
                Ok(())
            })?;
        }
        Ok(())
    })?;
    Ok(sources)
}

/// `unicode-range`ディスクリプタ: `<urange>`のカンマ区切りリスト
/// (`unicode-range: U+0-24F, U+1E00-1EFF;`)。`<urange>`自体の構文
/// (単一コードポイント/範囲/ワイルドカード)の妥当性検証は
/// `cssparser::UnicodeRange::parse`に委ねる(独自の型・
/// 手書きパーサは持たない)。1つでも不正なレンジがあれば
/// `parse_comma_separated`がクロージャの`Err`でリスト全体の
/// パースを打ち切るため、記述子全体が無効になる。
fn parse_unicode_range<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<UnicodeRange>, ParseError<'i, ()>> {
    input.parse_comma_separated(|input| {
        UnicodeRange::parse(input).map_err(|_| input.new_custom_error(()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::parse_stylesheet;
    use cssparser::ParserInput;

    fn parse_ranges(css: &str) -> Result<Vec<UnicodeRange>, ()> {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        parse_unicode_range(&mut parser).map_err(|_| ())
    }

    #[test]
    fn parses_a_single_code_point() {
        let ranges = parse_ranges("U+416").unwrap();
        assert_eq!(
            ranges,
            vec![UnicodeRange {
                start: 0x416,
                end: 0x416
            }]
        );
    }

    #[test]
    fn parses_an_explicit_range() {
        let ranges = parse_ranges("U+400-4ff").unwrap();
        assert_eq!(
            ranges,
            vec![UnicodeRange {
                start: 0x400,
                end: 0x4ff
            }]
        );
    }

    #[test]
    fn parses_a_wildcard_range() {
        let ranges = parse_ranges("U+4??").unwrap();
        assert_eq!(
            ranges,
            vec![UnicodeRange {
                start: 0x400,
                end: 0x4ff
            }]
        );
    }

    #[test]
    fn parses_comma_separated_ranges_in_source_order() {
        let ranges = parse_ranges("U+0-24F, U+1E00-1EFF, U+2C60-2C7F").unwrap();
        assert_eq!(
            ranges,
            vec![
                UnicodeRange {
                    start: 0x0,
                    end: 0x24F
                },
                UnicodeRange {
                    start: 0x1E00,
                    end: 0x1EFF
                },
                UnicodeRange {
                    start: 0x2C60,
                    end: 0x2C7F
                },
            ]
        );
    }

    #[test]
    fn rejects_a_range_whose_start_exceeds_its_end() {
        assert!(parse_ranges("U+4FF-400").is_err());
    }

    #[test]
    fn a_single_invalid_range_invalidates_the_whole_list() {
        // 1つ目は妥当だが2つ目が不正なので、リスト全体が無効になる
        // (`parse_comma_separated`がクロージャの`Err`で全体を打ち切るため)。
        assert!(parse_ranges("U+0-7F, not-a-range").is_err());
    }

    #[test]
    fn parses_family_and_url_src() {
        let sheet = parse_stylesheet(
            r#"@font-face { font-family: "My Brand"; src: url("fonts/brand.ttf"); }"#,
        );
        assert_eq!(sheet.font_faces.len(), 1);
        let rule = &sheet.font_faces[0];
        assert_eq!(rule.family, "My Brand");
        assert_eq!(
            rule.src,
            vec![FontFaceSource::Url("fonts/brand.ttf".to_string())]
        );
        assert_eq!(rule.weight, FontWeight::Normal);
        assert_eq!(rule.style, FontStyle::Normal);
        assert!(rule.unicode_range.is_empty());
    }

    #[test]
    fn parses_unicode_range_descriptor() {
        let sheet = parse_stylesheet(
            r#"@font-face { font-family: Brand; src: url("brand.ttf"); unicode-range: U+0-24F, U+1E00-1EFF; }"#,
        );
        let rule = &sheet.font_faces[0];
        assert_eq!(
            rule.unicode_range,
            vec![
                UnicodeRange {
                    start: 0x0,
                    end: 0x24F
                },
                UnicodeRange {
                    start: 0x1E00,
                    end: 0x1EFF
                },
            ]
        );
    }

    #[test]
    fn keeps_multiple_font_faces_with_different_unicode_ranges() {
        let sheet = parse_stylesheet(
            r#"
            @font-face { font-family: Brand; src: url("brand-latin.ttf"); unicode-range: U+0-24F; }
            @font-face { font-family: Brand; src: url("brand-cjk.ttf"); unicode-range: U+4E00-9FFF; }
            "#,
        );
        assert_eq!(sheet.font_faces.len(), 2);
        assert_eq!(
            sheet.font_faces[0].unicode_range,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x24F
            }]
        );
        assert_eq!(
            sheet.font_faces[1].unicode_range,
            vec![UnicodeRange {
                start: 0x4E00,
                end: 0x9FFF
            }]
        );
    }

    #[test]
    fn parses_unquoted_url_with_format_hint_and_weight_style() {
        let sheet = parse_stylesheet(
            "@font-face { font-family: Brand; src: url(brand-bold.ttf) format(\"truetype\"); font-weight: bold; font-style: italic; }",
        );
        let rule = &sheet.font_faces[0];
        assert_eq!(rule.family, "Brand");
        assert_eq!(
            rule.src,
            vec![FontFaceSource::Url("brand-bold.ttf".to_string())]
        );
        assert_eq!(rule.weight, FontWeight::Bold);
        assert_eq!(rule.style, FontStyle::Italic);
    }

    #[test]
    fn keeps_both_local_and_url_sources_in_order() {
        let sheet = parse_stylesheet(
            r#"@font-face { font-family: Brand; src: local("Brand Regular"), url("brand.ttf"); }"#,
        );
        let rule = &sheet.font_faces[0];
        assert_eq!(
            rule.src,
            vec![
                FontFaceSource::Local("Brand Regular".to_string()),
                FontFaceSource::Url("brand.ttf".to_string()),
            ]
        );
    }

    #[test]
    fn discards_the_rule_when_src_is_missing() {
        let sheet = parse_stylesheet(r#"@font-face { font-family: Brand; }"#);
        assert!(sheet.font_faces.is_empty());
    }

    #[test]
    fn discards_the_rule_when_font_family_is_missing() {
        let sheet = parse_stylesheet(r#"@font-face { src: url("brand.ttf"); }"#);
        assert!(sheet.font_faces.is_empty());
    }

    #[test]
    fn does_not_interfere_with_ordinary_style_rules() {
        let sheet = parse_stylesheet(
            r#"@font-face { font-family: Brand; src: url("brand.ttf"); } p { color: rgb(1, 2, 3); }"#,
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.rules.len(), 1);
    }
}
