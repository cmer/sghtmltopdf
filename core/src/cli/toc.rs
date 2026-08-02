//! 目次(`--toc`)のHTML組み立て。
//!
//! 生成する構造と既定スタイルはwkhtmltopdfの既定TOC XSL
//! (`src/lib/tocstylesheet.cc`)の出力に合わせてある。階層は入れ子の
//! `<ul>`で表現し、各項目は
//! `<li><div><a>見出し</a><span>ページ番号</span></div><ul>子</ul></li>`。
//! class属性は付けない(TOCは独立ドキュメントとしてレンダリングするため、
//! 要素セレクタが本文と衝突しない)。

use std::fmt::Write as _;

/// 目次1項目分。
#[derive(Debug, Clone, PartialEq)]
pub struct TocEntry {
    /// 見出しレベル(`h1`=1 … `h6`=6)。
    pub level: u8,
    pub title: String,
    /// 表示するページ番号。
    pub page: usize,
    /// リンク先の名前付き宛先。
    pub anchor: String,
    /// `--enable-toc-back-links`用に、この項目自身へ付ける宛先名。
    pub back_anchor: Option<String>,
}

/// 目次の見た目に関わるオプション(wkhtmltopdf互換)。
#[derive(Debug, Clone)]
pub struct TocOptions {
    pub header_text: String,
    /// `ul`のインデント(CSSの長さとしてそのまま書く)。
    pub level_indentation: String,
    /// 入れ子1段ごとの文字サイズ比(既定0.8)。
    pub text_size_shrink: f32,
    /// `div`に破線の下線を引くか。
    pub dotted_lines: bool,
    /// 目次→見出しのリンクを張るか。
    pub links: bool,
}

impl Default for TocOptions {
    fn default() -> Self {
        Self {
            header_text: "Table of Contents".to_string(),
            level_indentation: "1em".to_string(),
            text_size_shrink: 0.8,
            dotted_lines: true,
            links: true,
        }
    }
}

/// 目次のHTMLドキュメントを組み立てる。
pub fn build_toc_html(entries: &[TocEntry], options: &TocOptions) -> String {
    let mut html = String::from("<html><head><style>\n");
    let _ = write!(
        html,
        "h1 {{ text-align: center; font-size: 20px; }}\n\
         span {{ float: right; }}\n\
         li {{ list-style: none; }}\n\
         ul {{ font-size: 20px; padding-left: {}; }}\n\
         ul ul {{ font-size: {}%; }}\n\
         a {{ text-decoration: none; color: black; }}\n",
        options.level_indentation,
        (options.text_size_shrink * 100.0).round() as i32,
    );
    if options.dotted_lines {
        html.push_str("div { border-bottom: 1px dashed rgb(200,200,200); }\n");
    }
    html.push_str("</style></head><body>\n");
    let _ = writeln!(html, "<h1>{}</h1>", escape_html(&options.header_text));

    write_entries(&mut html, entries, options);

    html.push_str("</body></html>");
    html
}

/// 見出しレベルの相対関係で入れ子の`<ul>`を作る。
fn write_entries(html: &mut String, entries: &[TocEntry], options: &TocOptions) {
    if entries.is_empty() {
        html.push_str("<ul></ul>\n");
        return;
    }

    html.push_str("<ul>\n");
    // 現在開いている`<li>`のレベルを積む。
    let mut open_levels: Vec<u8> = Vec::new();

    for entry in entries {
        while let Some(&top) = open_levels.last() {
            if entry.level > top {
                // 深くなる: 子のリストを開く。
                html.push_str("<ul>\n");
                break;
            }
            // 同じか浅い: 開いている項目を閉じる。
            html.push_str("</li>\n");
            open_levels.pop();
            if let Some(&next_top) = open_levels.last() {
                if entry.level > next_top {
                    break;
                }
                html.push_str("</ul>\n");
            }
        }

        write_entry(html, entry, options);
        open_levels.push(entry.level);
    }

    // 残りを閉じる。
    while open_levels.pop().is_some() {
        html.push_str("</li>\n");
        if !open_levels.is_empty() {
            html.push_str("</ul>\n");
        }
    }
    html.push_str("</ul>\n");
}

fn write_entry(html: &mut String, entry: &TocEntry, options: &TocOptions) {
    html.push_str("<li><div>");
    let title = escape_html(&entry.title);
    if options.links {
        let mut attrs = format!(" href=\"#{}\"", escape_html(&entry.anchor));
        if let Some(back) = &entry.back_anchor {
            let _ = write!(attrs, " id=\"{}\"", escape_html(back));
        }
        let _ = write!(html, "<a{attrs}>{title}</a>");
    } else if let Some(back) = &entry.back_anchor {
        let _ = write!(html, "<a id=\"{}\">{title}</a>", escape_html(back));
    } else {
        html.push_str(&title);
    }
    let _ = write!(html, "<span>{}</span></div>", entry.page);
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: u8, title: &str, page: usize) -> TocEntry {
        TocEntry {
            level,
            title: title.to_string(),
            page,
            anchor: format!("a{page}"),
            back_anchor: None,
        }
    }

    #[test]
    fn the_default_style_matches_the_wkhtmltopdf_defaults() {
        let html = build_toc_html(&[entry(1, "x", 1)], &TocOptions::default());
        assert!(html.contains("h1 { text-align: center; font-size: 20px; }"));
        assert!(html.contains("span { float: right; }"));
        assert!(html.contains("li { list-style: none; }"));
        assert!(html.contains("ul { font-size: 20px; padding-left: 1em; }"));
        assert!(html.contains("ul ul { font-size: 80%; }"));
        assert!(html.contains("a { text-decoration: none; color: black; }"));
        assert!(html.contains("div { border-bottom: 1px dashed rgb(200,200,200); }"));
        assert!(html.contains("<h1>Table of Contents</h1>"));
    }

    #[test]
    fn an_entry_uses_the_div_a_span_structure() {
        let html = build_toc_html(&[entry(1, "はじめに", 3)], &TocOptions::default());
        assert!(
            // `"#`を含むためraw stringは`r##`で囲む。
            html.contains(r##"<li><div><a href="#a3">はじめに</a><span>3</span></div>"##),
            "got: {html}"
        );
    }

    #[test]
    fn deeper_levels_are_nested_in_child_uls() {
        let html = build_toc_html(
            &[entry(1, "A", 1), entry(2, "A-1", 2), entry(1, "B", 3)],
            &TocOptions::default(),
        );
        // A の下に子 <ul> が開き、B の前に閉じる。
        let a = html.find("A</a>").unwrap();
        let child_ul = html[a..].find("<ul>").unwrap() + a;
        let a1 = html.find("A-1</a>").unwrap();
        let b = html.find("B</a>").unwrap();
        assert!(
            child_ul < a1,
            "child <ul> must open before the nested entry"
        );
        assert!(a1 < b);
        // 閉じタグの数が釣り合っている。
        assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
        assert_eq!(html.matches("<li>").count(), html.matches("</li>").count());
    }

    #[test]
    fn a_level_jump_counts_as_one_nesting_step() {
        // h1 -> h3 の飛びも1段だけ深くする。
        let html = build_toc_html(
            &[entry(1, "A", 1), entry(3, "A-x", 2)],
            &TocOptions::default(),
        );
        assert_eq!(html.matches("<ul>").count(), 2);
        assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
    }

    #[test]
    fn options_change_the_generated_css_and_links() {
        let options = TocOptions {
            header_text: "目次".to_string(),
            level_indentation: "2em".to_string(),
            text_size_shrink: 0.5,
            dotted_lines: false,
            links: false,
        };
        let html = build_toc_html(&[entry(1, "A", 1)], &options);
        assert!(html.contains("<h1>目次</h1>"));
        assert!(html.contains("padding-left: 2em;"));
        assert!(html.contains("ul ul { font-size: 50%; }"));
        assert!(!html.contains("border-bottom"));
        assert!(!html.contains("<a href"), "links must be disabled: {html}");
        assert!(
            html.contains("<li><div>A<span>1</span></div>"),
            "got: {html}"
        );
    }

    #[test]
    fn back_links_put_an_id_on_the_toc_entry() {
        let mut e = entry(1, "A", 1);
        e.back_anchor = Some("__sgtocback_0".to_string());
        let html = build_toc_html(&[e], &TocOptions::default());
        assert!(html.contains(r#"id="__sgtocback_0""#), "got: {html}");
    }

    #[test]
    fn html_special_characters_are_escaped() {
        let html = build_toc_html(&[entry(1, "a<b>&\"c\"", 1)], &TocOptions::default());
        assert!(html.contains("a&lt;b&gt;&amp;&quot;c&quot;"), "got: {html}");
    }

    #[test]
    fn no_entries_still_produces_a_valid_document() {
        let html = build_toc_html(&[], &TocOptions::default());
        assert!(html.contains("<ul></ul>"));
        assert!(html.ends_with("</body></html>"));
    }
}
