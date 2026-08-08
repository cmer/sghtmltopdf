//! `content`(attr()/counter()/counters())/CSSカウンタ/`quotes`/`::first-letter`の
//! E2Eテスト。
//!
//! `list_style.rs`/`box_model.rs`と同じ方針: 実際のパイプライン(HTMLパース→
//! スタイルカスケード→レイアウト→PDFエンコード)を通して回帰を検知する。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, paginate_document, LaidOutBox, LaidOutContent, LineBox,
    PageSettings,
};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn test_fonts() -> FontCollection {
    FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ])
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn build_pdf(html_src: &str, css: &str) -> (usize, Vec<u8>) {
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let settings = PageSettings::default();

    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let engine_page_count = pages.len();
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);

    (engine_page_count, bytes)
}

/// レイアウト済みツリーを走査し、行に含まれるテキストを出現順に連結して返す
/// (マーカーは対象外。生成コンテンツも通常のスパンと同じ行に載るため、
/// このヘルパーだけで`::before`/`::after`/`::first-letter`いずれも検証できる)。
fn extract_text(b: &LaidOutBox) -> String {
    fn walk(b: &LaidOutBox, out: &mut String) {
        match &b.content {
            LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
                for child in children {
                    walk(child, out);
                }
            }
            LaidOutContent::Grid(grid) => {
                for child in grid.rows.iter().flat_map(|row| &row.items) {
                    walk(child, out);
                }
            }
            LaidOutContent::Inline(lines) => {
                // 同じインラインボックス内で折り返された行同士は、元は空白1つで
                // 繋がっていたテキストなので、行の間に空白を補う。一方、別々の
                // ボックス(ブロック境界)の間には補わない(ブロック境界は改行に
                // 相当し、空白ではないため)。
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    push_line(line, out);
                }
            }
            LaidOutContent::Table(table) => {
                if let Some(caption) = &table.caption {
                    walk(caption, out);
                }
                for row in &table.rows {
                    for cell in &row.cells {
                        walk(cell, out);
                    }
                }
            }
            LaidOutContent::Image(_) => {}
        }
    }
    fn push_line(line: &LineBox, out: &mut String) {
        // 単語間の空白はラン間の隙間として表現され、`run.text`には含まれない
        // (`layout::inline`の仕様)。x_offsetの隙間から単語境界の空白を復元する。
        let mut prev_end: Option<f32> = None;
        for run in &line.runs {
            if let Some(end) = prev_end {
                if run.x_offset > end + 0.01 {
                    out.push(' ');
                }
            }
            out.push_str(&run.text);
            prev_end = Some(run.x_offset + run.width);
        }
    }
    let mut out = String::new();
    walk(b, &mut out);
    out
}

fn layout(html_src: &str, css: &str) -> LaidOutBox {
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let tree = build_box_tree(&dom, &styles);
    layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    )
}

#[test]
fn content_attr_reads_the_element_own_attribute_value() {
    let laid = layout(
        r#"<div class="label" data-status="Active">x</div>"#,
        r#".label::before { content: attr(data-status) ": "; }"#,
    );
    assert_eq!(extract_text(&laid), "Active: x");
}

#[test]
fn counter_reset_on_an_element_stays_visible_to_its_following_siblings() {
    // 回帰テスト: h2のcounter-resetが自分の処理直後にpopされてしまうと、
    // 2つ目以降のh3が参照するsection値が消えてしまっていた。
    let laid = layout(
        r#"<h2>Intro</h2><h3>Background</h3><h3>Motivation</h3>"#,
        "h2 { counter-reset: section; } \
         h3 { counter-increment: section; } \
         h3::before { content: counter(section) \". \"; }",
    );
    // ::beforeは自身のテキストより前に来るので、"1. Background"の順になる。
    assert_eq!(extract_text(&laid), "Intro1. Background2. Motivation");
}

#[test]
fn nested_counters_are_scoped_per_ancestor_and_joined_with_the_separator() {
    let laid = layout(
        r#"
        <ol class="custom">
          <li>First
            <ol>
              <li>Nested A</li>
              <li>Nested B</li>
            </ol>
          </li>
          <li>Second</li>
        </ol>
        "#,
        "ol.custom { counter-reset: item; } \
         ol.custom li { counter-increment: item; } \
         ol.custom li::before { content: counters(item, \".\") \" \"; } \
         ol.custom li ol { counter-reset: item; }",
    );
    let text = extract_text(&laid);
    // 最初のliは入れ子の<ol>(ブロック要素)を子に持つため、::before自体が
    // 非対応(box_tree.rsの「ブロック子を持つ要素では::before/::after非対応」
    // という簡略化)。よってプレフィックスなしの"First"のままになる。
    assert!(text.contains("First"), "text was: {text:?}");
    assert!(!text.contains("1. First"), "text was: {text:?}");
    assert!(text.contains("1.1 Nested A"), "text was: {text:?}");
    assert!(text.contains("1.2 Nested B"), "text was: {text:?}");
    // 入れ子scopeは"First"を含むliが抜ける時点でpopされるため、Secondは
    // 外側のitemカウンタ(2)のみを見る。
    assert!(text.contains("2 Second"), "text was: {text:?}");
}

#[test]
fn after_content_is_resolved_after_descendants_so_it_reflects_their_counter_changes() {
    // pにブロック子孫を混ぜると::after自体が非対応になるため、子孫は
    // インライン要素(span)にする。
    let laid = layout(
        r#"<p class="section">Heading <span class="mark">note</span></p>"#,
        "p.section { counter-reset: n; } \
         span.mark { counter-increment: n; } \
         p.section::after { content: \" total: \" counter(n); }",
    );
    assert_eq!(extract_text(&laid), "Heading note total: 1");
}

#[test]
fn nested_quotes_use_the_pair_matching_their_nesting_depth() {
    let laid = layout(
        r#"<p>She said <q>hello <q>nested</q> world</q> to everyone.</p>"#,
        "q { quotes: \"\\201C\" \"\\201D\" \"\\2018\" \"\\2019\"; } \
         q::before { content: open-quote; } \
         q::after { content: close-quote; }",
    );
    let text = extract_text(&laid);
    assert_eq!(
        text,
        "She said \u{201C}hello \u{2018}nested\u{2019} world\u{201D} to everyone."
    );
}

#[test]
fn first_letter_splits_only_the_first_character_into_its_own_styled_run() {
    let laid = layout(
        r#"<p class="dropcap">Hello world</p>"#,
        "p.dropcap::first-letter { font-size: 2.5em; }",
    );
    let base_font_size = sghtmltopdf_core::style::ComputedStyle::default()
        .font_size
        .0;

    fn first_line(b: &LaidOutBox) -> Option<&LineBox> {
        match &b.content {
            LaidOutContent::Inline(lines) => lines.first(),
            LaidOutContent::Blocks(children) => children.iter().find_map(first_line),
            _ => None,
        }
    }
    let line = first_line(&laid).expect("expected inline content");
    assert_eq!(line.runs[0].text, "H");
    assert_eq!(line.runs[0].font_size, base_font_size * 2.5);
    assert!(line.runs[1..].iter().all(|r| r.font_size == base_font_size));
}

#[test]
fn all_generated_content_features_combined_render_a_valid_pdf_end_to_end() {
    let html_src = r#"
        <body>
        <h2>Introduction</h2>
        <h3>Background</h3>
        <h3>Motivation</h3>
        <h2>Methods</h2>
        <h3>Setup</h3>

        <div class="attr-test" data-label="Status">Active</div>

        <p>She said <q>hello <q>nested</q> world</q> to everyone.</p>

        <p class="dropcap">This paragraph has a drop cap.</p>

        <ol class="custom">
          <li>First
            <ol>
              <li>Nested A</li>
              <li>Nested B</li>
            </ol>
          </li>
          <li>Second</li>
        </ol>
        </body>
    "#;
    let css = "
        body { counter-reset: chapter; margin: 0; }
        h2 { counter-reset: section; counter-increment: chapter; }
        h2::before { content: \"Chapter \" counter(chapter) \": \"; }
        h3 { counter-increment: section; }
        h3::before { content: counter(chapter) \".\" counter(section) \" \"; }
        .attr-test::before { content: attr(data-label) \": \"; }
        q { quotes: \"\\201C\" \"\\201D\" \"\\2018\" \"\\2019\"; }
        q::before { content: open-quote; }
        q::after { content: close-quote; }
        p.dropcap::first-letter { font-size: 2.5em; font-weight: bold; color: #a33; }
        ol.custom { counter-reset: item; }
        ol.custom li { counter-increment: item; }
        ol.custom li::before { content: counters(item, \".\") \" \"; }
        ol.custom li ol { counter-reset: item; }
    ";
    let (page_count, bytes) = build_pdf(html_src, css);
    assert!(page_count >= 1);
    assert!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0,
        "the font should be embedded to render the text"
    );
}
