//! `@import`文の検出・再帰展開(パース前のテキスト前処理)。
//!
//! `parse_stylesheet`本体にI/Oを持ち込まず、パース前のCSSテキストに対する
//! 文字列レベルの展開として実装する。`@import`文の検出はCSS仕様上の
//! 「先頭にしか書けない」規定を厳密にvalidateせず、CSS内のどこにあっても
//! 検出・展開する(スパイクで非先頭のimportも正しく検出できることを確認済み)。
//! 展開結果は、`@import`文があった位置にそのままフェッチ内容を差し込む
//! (hoistして先頭にまとめるのではない、真の意味でのin-place置換)。

use std::ops::Range;

use cssparser::{Delimiter, Parser, ParserInput, Token};

use crate::img::{DocumentImageCache, ImageFetcher};

/// 循環import対策の再帰深さ上限。URL正規化による訪問済み集合の判定は相対/絶対
/// /data:混在下でのコストが見合わないため、単純な深さ上限で代替する。
const MAX_IMPORT_DEPTH: u32 = 16;

struct ImportStatement {
    href: String,
    /// 文全体(`@import`から終端の`;`まで)の元cssにおけるバイト範囲。
    range: Range<usize>,
}

/// `css`中の`@import`文を検出し、フェッチした内容で再帰的に展開したCSS
/// テキストを返す。フェッチ・デコードに失敗した`@import`、または
/// [`MAX_IMPORT_DEPTH`]を超えた`@import`は、その1件だけ無視して標準エラー
/// 出力に警告を出し、処理を継続する(画像・外部スタイルシートと同じ方針)。
pub fn resolve_imports(
    css: &str,
    fetcher: &ImageFetcher,
    cache: &DocumentImageCache,
    depth: u32,
) -> String {
    let imports = find_imports(css);
    if imports.is_empty() {
        return css.to_string();
    }

    let mut result = String::with_capacity(css.len());
    let mut cursor = 0usize;

    for import in &imports {
        result.push_str(&css[cursor..import.range.start]);
        cursor = import.range.end;

        if depth >= MAX_IMPORT_DEPTH {
            eprintln!(
                "警告: @importの再帰が深すぎるため無視しました(上限{MAX_IMPORT_DEPTH}階層): {}",
                import.href
            );
            continue;
        }

        match cache.get_or_fetch(fetcher, &import.href) {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    result.push_str(&resolve_imports(text, fetcher, cache, depth + 1));
                    result.push('\n');
                }
                Err(_) => eprintln!(
                    "警告: @importで取得したCSSがUTF-8として解釈できません: {}",
                    import.href
                ),
            },
            Err(e) => eprintln!("警告: @importの取得に失敗しました: {}: {e}", import.href),
        }
    }
    result.push_str(&css[cursor..]);
    result
}

/// `css`中の`@import`文をトークン走査で検出する(I/Oは行わない)。
fn find_imports(css: &str) -> Vec<ImportStatement> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut found = Vec::new();

    loop {
        let start_state = parser.state();
        match parser.next_including_whitespace_and_comments() {
            Ok(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("import") => {
                // メディアクエリ付き(`@import url(...) screen;`)であっても、
                // hrefだけを取り出しメディア部分は無条件import扱いで捨てる
                // (`@media`自体が非対応スコープのため)。
                let href = parser
                    .parse_until_before::<_, _, ()>(Delimiter::Semicolon, |input| {
                        input
                            .expect_url_or_string()
                            .map(|s| s.as_ref().to_string())
                            .map_err(|_| input.new_custom_error(()))
                    })
                    .ok();
                let _ = parser.next(); // 終端の`;`(あれば)まで読み飛ばす。
                let end = parser.position().byte_index();
                if let Some(href) = href {
                    found.push(ImportStatement {
                        href,
                        range: start_state.position().byte_index()..end,
                    });
                }
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn no_remote_fetcher() -> ImageFetcher {
        ImageFetcher::new(PathBuf::from("."), false)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-style-import-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn leaves_css_without_import_unchanged() {
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();
        let css = "p { color: red; }";
        assert_eq!(resolve_imports(css, &fetcher, &cache, 0), css);
    }

    #[test]
    fn splices_imported_content_in_place() {
        let dir = temp_dir("splices_in_place");
        std::fs::write(dir.join("other.css"), b"p { color: blue; }").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let css = r#"a { color: green; } @import url("other.css"); div { color: red; }"#;
        let expanded = resolve_imports(css, &fetcher, &cache, 0);

        let a_pos = expanded.find("a {").unwrap();
        let p_pos = expanded.find("p {").unwrap();
        let div_pos = expanded.find("div {").unwrap();
        assert!(
            a_pos < p_pos && p_pos < div_pos,
            "imported content should be spliced exactly where the @import statement was, \
             not hoisted to the front: {expanded:?}"
        );
        assert!(!expanded.contains("@import"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recursively_expands_nested_imports() {
        let dir = temp_dir("nested");
        std::fs::write(
            dir.join("a.css"),
            br#"@import url("b.css"); a { color: red; }"#,
        )
        .unwrap();
        std::fs::write(dir.join("b.css"), b"b { color: blue; }").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let css = r#"@import url("a.css");"#;
        let expanded = resolve_imports(css, &fetcher, &cache, 0);
        assert!(expanded.contains("b { color: blue; }"));
        assert!(expanded.contains("a { color: red; }"));
        assert!(!expanded.contains("@import"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_circular_import_is_guarded_by_the_depth_limit_without_hanging() {
        let dir = temp_dir("circular");
        std::fs::write(
            dir.join("a.css"),
            br#"@import url("b.css"); a { color: red; }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("b.css"),
            br#"@import url("a.css"); b { color: blue; }"#,
        )
        .unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        // 無限再帰にならず、MAX_IMPORT_DEPTHで打ち切られて終了することを確認する。
        let expanded = resolve_imports(r#"@import url("a.css");"#, &fetcher, &cache, 0);
        assert!(expanded.contains("a { color: red; }"));
        assert!(expanded.contains("b { color: blue; }"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_import_is_skipped_without_panicking() {
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();
        let css = r#"@import url("does-not-exist.css"); p { color: red; }"#;
        let expanded = resolve_imports(css, &fetcher, &cache, 0);
        assert_eq!(expanded.trim(), "p { color: red; }");
    }
}
