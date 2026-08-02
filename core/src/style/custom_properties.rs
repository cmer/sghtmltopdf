//! CSS Custom Properties(`--foo`/`var()`)の、パース前テキスト置換による対応。
//!
//! `style/import.rs::resolve_imports`(`@import`展開)と同じ「トークン走査で
//! バイト範囲を特定し、元テキストから置換後の文字列を再構築する」パターンを
//! 使う。cascade/継承ベースの実装ではなく、文書全体でフラットな名前空間の
//! 単純なテキスト置換であることに注意。`parse_stylesheet`本体はこの
//! モジュールを経由した後のテキストを受け取るため、`var()`も
//! カスタムプロパティも一切知らないままでよい。

use std::collections::HashMap;

use cssparser::{ParseError, Parser, ParserInput, Token};

/// `declared`自身に含まれる`var()`の解決・文書全体への適用それぞれで安定する
/// まで繰り返す反復回数の上限(`MAX_IMPORT_DEPTH`と同じ考え方)。
const MAX_SUBSTITUTION_ITERATIONS: u32 = 8;

/// `css`中の`--foo: value;`宣言・`var(--foo, fallback)`呼び出しをすべて
/// テキストとして解決した後のCSSテキストを返す。
pub fn substitute_custom_properties(css: &str) -> String {
    let mut declared = collect_custom_properties(css);

    // 他のカスタムプロパティを参照するカスタムプロパティ(`--b: var(--a)`)を、
    // 宣言順に関係なく解決できるよう安定するまで解決する。
    for _ in 0..MAX_SUBSTITUTION_ITERATIONS {
        let mut changed = false;
        let next: HashMap<String, String> = declared
            .iter()
            .map(|(name, value)| {
                let substituted = substitute_var_calls(value, &declared);
                changed |= substituted != *value;
                (name.clone(), substituted)
            })
            .collect();
        declared = next;
        if !changed {
            break;
        }
    }

    // 文書全体へ適用する。フォールバック値の中に別の`var()`が残っているケース
    // (`var(--a, var(--b))`で`--a`が未定義の場合)も解決できるよう、安定する
    // まで繰り返す。
    let mut result = css.to_string();
    for _ in 0..MAX_SUBSTITUTION_ITERATIONS {
        let next = substitute_var_calls(&result, &declared);
        if next == result {
            break;
        }
        result = next;
    }
    result
}

/// トークンがブロック開始(`{`/`(`/`[`/`func(`)であれば、`cssparser`の
/// 仕様上その中身は`Parser::parse_nested_block`で明示的に入らない限り
/// 見えない(次の`next()`呼び出しで自動的にブロック終端まで読み飛ばされる)。
/// `--foo: value`は`{ }`ルール本体の中にしかない(`@page`/`@media`等の
/// at-ruleブロックも含む)ため、収集・置換のどちらも全ブロック型に対して
/// 再帰的に降りる必要がある。
fn is_block_start(token: &Token) -> bool {
    matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    )
}

/// `css`中の`--foo: value;`宣言を走査して収集する(トークン走査、I/Oなし)。
/// 同名の宣言が複数あれば、テキスト出現順で最後のものが勝つ(セレクタの
/// 詳細度・オリジンは見ない)。
fn collect_custom_properties(css: &str) -> HashMap<String, String> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut declared = HashMap::new();
    collect_custom_properties_in_scope(&mut parser, css, &mut declared);
    declared
}

/// `parser`の現在のスコープ(文書全体、または`parse_nested_block`で入った
/// ブロックの内部)を走査する。ブロック開始トークンに遭遇したら再帰的に
/// 中身も走査する。
fn collect_custom_properties_in_scope(
    parser: &mut Parser,
    css: &str,
    declared: &mut HashMap<String, String>,
) {
    loop {
        match parser.next() {
            Ok(Token::Ident(name)) if name.starts_with("--") => {
                let name = name.to_string();
                if parser.try_parse(|input| input.expect_colon()).is_err() {
                    continue;
                }
                let value_start = parser.position().byte_index();
                let value_end = loop {
                    let state = parser.state();
                    match parser.next() {
                        Ok(Token::Semicolon) => break state.position().byte_index(),
                        Ok(Token::CloseCurlyBracket) => {
                            // このブロックの終端`}`は消費せず、外側のループへ戻す。
                            parser.reset(&state);
                            break state.position().byte_index();
                        }
                        Ok(token) if is_block_start(token) => {
                            // ここで即座に消費してしまう(`continue`で先送りに
                            // すると、次のイテレーションで捕まえる`state()`が
                            // 保留中のブロックスキップより前の不正確な位置に
                            // なってしまうため)。値の一部としての中身は
                            // 素通りしてよいので、単に最後まで読み飛ばす。
                            let _ = parser.parse_nested_block(
                                |input| -> Result<(), ParseError<'_, ()>> {
                                    while input.next().is_ok() {}
                                    Ok(())
                                },
                            );
                            continue;
                        }
                        Ok(_) => continue,
                        Err(_) => break parser.position().byte_index(),
                    }
                };
                let value = css[value_start..value_end].trim();
                if !value.is_empty() {
                    declared.insert(name, value.to_string());
                }
            }
            Ok(token) if is_block_start(token) => {
                let _ = parser.parse_nested_block(|input| -> Result<(), ParseError<'_, ()>> {
                    collect_custom_properties_in_scope(input, css, declared);
                    Ok(())
                });
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

/// `css`中の`var(--foo)`/`var(--foo, fallback)`を、`declared`を使って1回分
/// テキスト置換する(ネストしたフォールバック中の`var()`解決は
/// [`substitute_custom_properties`]側の反復に任せる)。
fn substitute_var_calls(css: &str, declared: &HashMap<String, String>) -> String {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut result = String::with_capacity(css.len());
    let mut cursor = 0usize;
    substitute_var_calls_in_scope(&mut parser, css, declared, &mut result, &mut cursor);
    result.push_str(&css[cursor..]);
    result
}

/// `parser`の現在のスコープを走査し、見つけた`var()`の置換テキストを
/// `result`へ書き出す(`cursor`は`css`中で未書き出しの開始位置)。
/// ブロック開始トークンに遭遇したら再帰的に中身も処理する(`{`/`(`/`[`
/// 自体はここでは書き出さず、`cursor`を進めないままにすることで、次の
/// 書き出しタイミングでまとめて元テキストのまま流れるようにする)。
fn substitute_var_calls_in_scope(
    parser: &mut Parser,
    css: &str,
    declared: &HashMap<String, String>,
    result: &mut String,
    cursor: &mut usize,
) {
    loop {
        let start_state = parser.state();
        match parser.next_including_whitespace_and_comments() {
            Ok(Token::Function(fn_name)) if fn_name.eq_ignore_ascii_case("var") => {
                let call_start = start_state.position().byte_index();
                let mut fallback_range: Option<(usize, usize)> = None;
                let var_name =
                    parser.parse_nested_block(|input| -> Result<String, ParseError<'_, ()>> {
                        let name = input.expect_ident()?.as_ref().to_string();
                        if input.try_parse(|input| input.expect_comma()).is_ok() {
                            let start = input.position().byte_index();
                            while input.next().is_ok() {}
                            let end = input.position().byte_index();
                            fallback_range = Some((start, end));
                        }
                        Ok(name)
                    });
                let call_end = parser.position().byte_index();

                result.push_str(&css[*cursor..call_start]);
                let replacement = match var_name {
                    Ok(name) => match declared.get(&name) {
                        Some(value) => value.clone(),
                        None => match fallback_range {
                            Some((start, end)) => css[start..end].trim().to_string(),
                            // 未定義でフォールバックも無い場合は元の
                            // テキストのまま残す(後段のプロパティパーサが未知
                            // トークンとして黙って無視する)。
                            None => css[call_start..call_end].to_string(),
                        },
                    },
                    Err(_) => css[call_start..call_end].to_string(),
                };
                result.push_str(&replacement);
                *cursor = call_end;
            }
            Ok(token) if is_block_start(token) => {
                let _ = parser.parse_nested_block(|input| -> Result<(), ParseError<'_, ()>> {
                    substitute_var_calls_in_scope(input, css, declared, result, cursor);
                    Ok(())
                });
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_a_simple_custom_property() {
        let css = ":root { --main-color: red; } p { color: var(--main-color); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("color: red"), "{out}");
        assert!(!out.contains("var("), "{out}");
    }

    #[test]
    fn uses_fallback_when_the_custom_property_is_undefined() {
        let css = "p { color: var(--undefined, blue); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("color: blue"), "{out}");
    }

    #[test]
    fn leaves_unresolved_var_untouched_when_no_fallback_exists() {
        let css = "p { color: var(--undefined); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("var(--undefined)"), "{out}");
    }

    #[test]
    fn resolves_a_custom_property_that_references_another_one() {
        let css = ":root { --base: 8px; --gap: var(--base); } div { margin: var(--gap); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("margin: 8px"), "{out}");
    }

    #[test]
    fn later_declaration_wins_regardless_of_selector_scope() {
        let css = ".a { --x: 1px; } .b { --x: 2px; } p { width: var(--x); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("width: 2px"), "{out}");
    }

    #[test]
    fn preserves_surrounding_text_and_whitespace() {
        let css = "p {\n  color: var(--c, green);\n  font-size: 12px;\n}";
        let out = substitute_custom_properties(css);
        assert!(out.contains("font-size: 12px"), "{out}");
        assert!(out.contains("color: green"), "{out}");
    }

    #[test]
    fn resolves_nested_var_inside_a_fallback() {
        let css = ":root { --b: navy; } p { color: var(--a, var(--b)); }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("color: navy"), "{out}");
    }

    #[test]
    fn ignores_custom_property_like_text_inside_string_literals_and_comments() {
        let css = "p { content: \"--not-a-var: x;\"; /* --also-not: y; */ color: red; }";
        let out = substitute_custom_properties(css);
        assert!(out.contains("\"--not-a-var: x;\""), "{out}");
        assert!(out.contains("color: red"), "{out}");
    }
}
