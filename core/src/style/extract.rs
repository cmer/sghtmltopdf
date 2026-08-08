//! DOM中の`<style>`/`<link rel=stylesheet>`要素からauthorスタイルシートを
//! 組み立てる。
//!
//! `style="..."`属性(インラインスタイル)の抽出は未対応。
//!
//! DOM走査(I/O無し、[`collect_css_sources`])とhref解決(I/Oあり)を分離する。全
//! CSSソース(インライン・外部いずれも)をdocument順に連結してから
//! `parse_stylesheet`を1回だけ呼ぶため、フェッチした外部CSS内の相対`url()`は
//! 常に元HTMLの`base_dir`基準で解決される
//! (スタイルシートごとの基準切り替えは非対応)。
//!
//! 各CSSソースのテキストは、連結する前に[`resolve_imports`]で`@import`を
//! 再帰展開する。`parse_stylesheet`自体は`@import`を知らないまま(cssparserの
//! エラー回復で無視される)なので、`extract_author_stylesheet`を経由しない
//! 直接呼び出しでは従来通り`@import`は展開されず単に無視される。
//!
//! 連結後・パース前に[`substitute_custom_properties`]でCSS Custom
//! Properties(`--foo`/`var`)をテキスト置換で解決する(`<style>`/`<link>`をまた
//! いだ文書全体でフラットな名前空間として扱う)。

use crate::html::{is_stylesheet_link, Dom, NodeData, NodeId};
use crate::img::{DocumentImageCache, ImageFetcher};

use super::custom_properties::substitute_custom_properties;
use super::import::resolve_imports;
use super::stylesheet::{parse_stylesheet, Stylesheet};

/// DOM中のCSSソース1件(document順)。
#[derive(Debug, Clone, PartialEq, Eq)]
enum CssSource {
    /// `<style>`要素のテキスト内容。
    Inline(String),
    /// `<link rel=stylesheet href="...">`のhref(未解決の生の値)。
    External(String),
}

/// DOM中の全ての`<style>`/`<link rel=stylesheet>`のCSSを、document順を
/// 保ったまま連結してパースする。
///
/// 外部スタイルシート(`<link>`)の取得に失敗した場合(ネットワークエラー・SSRF
/// ブロック・非2xx・不正なUTF-8等、いずれも同列)は、画像と同じくその
/// スタイルシートだけを無視して標準エラー出力に警告を出し、処理全体は継続する
/// (壊れた/ブロックされたURLで文書生成全体を止めない)。
pub fn extract_author_stylesheet(
    dom: &Dom,
    fetcher: &ImageFetcher,
    cache: &DocumentImageCache,
) -> Stylesheet {
    let mut css = String::new();
    for source in collect_css_sources(dom) {
        match source {
            CssSource::Inline(text) => {
                css.push_str(&resolve_imports(&text, fetcher, cache, 0));
                css.push('\n');
            }
            CssSource::External(href) => match cache.get_or_fetch(fetcher, &href) {
                Ok(bytes) => match std::str::from_utf8(&bytes) {
                    Ok(text) => {
                        css.push_str(&resolve_imports(text, fetcher, cache, 0));
                        css.push('\n');
                    }
                    Err(_) => {
                        eprintln!(
                            "警告: 外部スタイルシートの取得は成功しましたが、UTF-8として解釈できません: {href}"
                        );
                    }
                },
                Err(e) => {
                    eprintln!("警告: 外部スタイルシートの取得に失敗しました: {href}: {e}");
                }
            },
        }
    }
    parse_stylesheet(&substitute_custom_properties(&css))
}

/// DOM木を1回走査し、document順を保ったまま「インラインCSSテキスト」か
/// 「`<link>`のhref」かを列挙する。I/Oは一切行わない(純粋なDOM走査)。
fn collect_css_sources(dom: &Dom) -> Vec<CssSource> {
    let mut sources = Vec::new();
    collect_css_sources_rec(dom, dom.document(), &mut sources);
    sources
}

fn collect_css_sources_rec(dom: &Dom, node: NodeId, out: &mut Vec<CssSource>) {
    if let NodeData::Element { name, attrs, .. } = &dom.node(node).data {
        if &*name.local == "style" {
            let mut text = String::new();
            for child in dom.children(node) {
                if let NodeData::Text { contents } = &dom.node(child).data {
                    text.push_str(contents);
                    text.push('\n');
                }
            }
            out.push(CssSource::Inline(text));
            return;
        }
        if &*name.local == "link" && is_stylesheet_link(attrs) {
            let href = attrs
                .iter()
                .find(|attr| &*attr.name.local == "href")
                .map(|attr| attr.value.to_string())
                .filter(|s| !s.is_empty());
            if let Some(href) = href {
                out.push(CssSource::External(href));
            }
            return; // <link>はvoid element(子を持たない)。
        }
    }
    for child in dom.children(node) {
        collect_css_sources_rec(dom, child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use std::path::PathBuf;

    fn no_remote_fetcher() -> ImageFetcher {
        ImageFetcher::new(PathBuf::from("."), false)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-style-extract-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_and_parses_style_tag_contents() {
        let dom = html::parse(
            br#"<html><head><style>p { color: rgb(1, 2, 3); }</style></head>
                <body><p>text</p></body></html>"#,
        );
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();
        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn combines_multiple_style_tags() {
        let dom = html::parse(
            br#"<html><head>
                <style>p { color: rgb(1, 2, 3); }</style>
                <style>div { color: rgb(4, 5, 6); }</style>
                </head><body></body></html>"#,
        );
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();
        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn returns_empty_stylesheet_when_no_style_tags() {
        let dom = html::parse(b"<p>text</p>");
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();
        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn fetches_and_parses_a_local_external_stylesheet() {
        let dir = temp_dir("fetches_local");
        std::fs::write(dir.join("main.css"), b"p { color: rgb(1, 2, 3); }").unwrap();
        let dom = html::parse(
            br#"<html><head><link rel="stylesheet" href="main.css"></head><body></body></html>"#,
        );
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn preserves_document_order_between_link_and_style() {
        // 後勝ちのカスケード順が保たれるよう、<link>と<style>の出現順
        // (この場合<link>が先)を維持したまま連結・パースされるはず。
        use super::super::values::SpecifiedLength;
        use crate::style::PropertyDeclaration;

        let dir = temp_dir("preserves_order");
        std::fs::write(dir.join("main.css"), b"p { font-size: 11px; }").unwrap();
        let dom = html::parse(
            br#"<html><head>
                <link rel="stylesheet" href="main.css">
                <style>p { font-size: 22px; }</style>
                </head><body></body></html>"#,
        );
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 2);

        let font_size_px = |decls: &[PropertyDeclaration]| match decls.first() {
            Some(PropertyDeclaration::FontSize(SpecifiedLength::Px(px))) => *px,
            other => panic!("expected a single font-size: Px(_) declaration, got {other:?}"),
        };
        assert_eq!(
            font_size_px(&sheet.rules[0].declarations),
            11.0,
            "the <link> (appearing first) should parse first"
        );
        assert_eq!(
            font_size_px(&sheet.rules[1].declarations),
            22.0,
            "the <style> (appearing second) should parse second, so it wins the cascade"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_external_stylesheet_is_skipped_without_panicking() {
        let dom = html::parse(
            br#"<html><head><link rel="stylesheet" href="does-not-exist.css"></head>
                <body></body></html>"#,
        );
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn ignores_a_link_that_is_not_a_stylesheet() {
        let dom = html::parse(
            br#"<html><head><link rel="icon" href="favicon.ico"></head><body></body></html>"#,
        );
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn ignores_a_stylesheet_link_with_no_href() {
        let dom = html::parse(br#"<html><head><link rel="stylesheet"></head><body></body></html>"#);
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn expands_at_import_inside_a_style_tag() {
        let dir = temp_dir("import_in_style_tag");
        std::fs::write(dir.join("imported.css"), b"p { color: rgb(1, 2, 3); }").unwrap();
        let dom = html::parse(
            br#"<html><head><style>@import url("imported.css"); div { color: rgb(4, 5, 6); }</style></head>
                <body></body></html>"#,
        );
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn expands_at_import_inside_a_fetched_external_stylesheet() {
        let dir = temp_dir("import_in_external");
        std::fs::write(
            dir.join("main.css"),
            br#"@import url("base.css"); p { color: rgb(1, 2, 3); }"#,
        )
        .unwrap();
        std::fs::write(dir.join("base.css"), b"div { color: rgb(4, 5, 6); }").unwrap();
        let dom = html::parse(
            br#"<html><head><link rel="stylesheet" href="main.css"></head><body></body></html>"#,
        );
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_at_import_is_skipped_without_failing_the_whole_stylesheet() {
        let dom = html::parse(
            br#"<html><head><style>@import url("does-not-exist.css"); p { color: rgb(1, 2, 3); }</style></head>
                <body></body></html>"#,
        );
        let fetcher = no_remote_fetcher();
        let cache = DocumentImageCache::new();

        let sheet = extract_author_stylesheet(&dom, &fetcher, &cache);
        assert_eq!(sheet.rules.len(), 1);
    }
}
