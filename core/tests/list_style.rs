//! `list-style-type`/`list-style-position`/`list-style-image`/`list-style`
//! ショートハンドのE2Eテスト(M8 Phase 1 Lists)。
//!
//! `typography.rs`/`table_caption.rs`と同じ方針: 実際のパイプライン(HTMLパース→
//! スタイルカスケード→ページ分割→PDFエンコード)を通して回帰を検知する。
//! マーカーの座標・カウンタ挙動の詳細な検証は`layout_document`(ページ分割前)の
//! 結果に対して行い、PDFエンコードまでのパイプライン全体がクラッシュせず
//! 妥当な出力になることは`build_pdf`で別途確認する。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, paginate_document, LaidOutBox, LaidOutContent, PageSettings,
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

fn page_count_in_pdf(bytes: &[u8]) -> usize {
    count_occurrences(bytes, b"/MediaBox")
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
    assert_eq!(
        page_count_in_pdf(&bytes),
        engine_page_count,
        "PDF page count should match the layout engine's own page count"
    );

    (engine_page_count, bytes)
}

fn find_tag(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == tag {
            return Some(id);
        }
    }
    dom.children(id).find_map(|child| find_tag(dom, child, tag))
}

fn find_all_tags(dom: &Dom, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == tag {
            out.push(id);
        }
    }
    for child in dom.children(id) {
        find_all_tags(dom, child, tag, out);
    }
}

fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
    if b.node == Some(target) {
        return Some(b);
    }
    if let LaidOutContent::Blocks(children) = &b.content {
        for child in children {
            if let Some(found) = find_laid_out(child, target) {
                return Some(found);
            }
        }
    }
    None
}

/// `layout_document`まで(ページ分割前)を実行する共通ヘルパー。
fn layout(html_src: &str, css: &str) -> (Dom, LaidOutBox) {
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let tree = build_box_tree(&dom, &styles);
    let laid = layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    );
    (dom, laid)
}

#[test]
fn unordered_list_renders_a_valid_pdf_with_disc_markers() {
    let html_src = r#"<ul><li>one</li><li>two</li></ul>"#;
    let (page_count, bytes) = build_pdf(html_src, "body { margin: 0; }");
    assert_eq!(page_count, 1);
    assert!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0,
        "the font should be embedded to render the list text and markers"
    );
}

#[test]
fn ordered_list_numbers_items_in_document_order() {
    let (dom, laid) = layout(
        r#"<ol><li>a</li><li>b</li><li>c</li></ol>"#,
        "body { margin: 0; }",
    );
    let mut lis = Vec::new();
    find_all_tags(&dom, dom.document(), "li", &mut lis);
    assert_eq!(lis.len(), 3);

    let expected = ["1.", "2.", "3."];
    for (li, expected_marker) in lis.iter().zip(expected) {
        let li_box = find_laid_out(&laid, *li).expect("li box not found");
        assert_eq!(
            li_box.marker.as_ref().map(|m| m.runs[0].text.as_str()),
            Some(expected_marker)
        );
    }
}

#[test]
fn ol_start_attribute_offsets_the_numbering_end_to_end() {
    let (dom, laid) = layout(
        r#"<ol start="10"><li>a</li><li>b</li></ol>"#,
        "body { margin: 0; }",
    );
    let mut lis = Vec::new();
    find_all_tags(&dom, dom.document(), "li", &mut lis);

    let first = find_laid_out(&laid, lis[0]).expect("li box not found");
    assert_eq!(first.marker.as_ref().unwrap().runs[0].text, "10.");
    let second = find_laid_out(&laid, lis[1]).expect("li box not found");
    assert_eq!(second.marker.as_ref().unwrap().runs[0].text, "11.");
}

#[test]
fn nested_ordered_list_restarts_numbering_and_indents_further_than_its_parent() {
    let (dom, laid) = layout(
        r#"<ol><li>outer</li><li><ol><li>inner</li></ol></li></ol>"#,
        "body { margin: 0; }",
    );
    let mut lis = Vec::new();
    find_all_tags(&dom, dom.document(), "li", &mut lis);
    assert_eq!(lis.len(), 3, "2 top-level li + 1 nested li");

    let outer_first = find_laid_out(&laid, lis[0]).expect("outer li not found");
    let outer_second = find_laid_out(&laid, lis[1]).expect("outer li (with nested ol) not found");
    let inner = find_laid_out(&laid, lis[2]).expect("inner li not found");

    assert_eq!(outer_first.marker.as_ref().unwrap().runs[0].text, "1.");
    assert_eq!(outer_second.marker.as_ref().unwrap().runs[0].text, "2.");
    // 入れ子の`<ol>`は独立したカウンタスコープを持つため1から数え直す。
    assert_eq!(inner.marker.as_ref().unwrap().runs[0].text, "1.");

    // 入れ子の`<ol>`自身の`padding-left: 40px`(UAスタイルシート)により、
    // 内側のマーカーは外側のマーカーよりcontent edgeがさらに右にあるはず。
    let outer_marker = outer_first.marker.as_ref().unwrap();
    let inner_marker = inner.marker.as_ref().unwrap();
    assert!(
        inner_marker.rect.x > outer_marker.rect.x,
        "nested list marker (x={}) should sit further right than the outer one (x={})",
        inner_marker.rect.x,
        outer_marker.rect.x
    );
}

#[test]
fn list_style_type_none_still_advances_the_counter_but_has_no_visible_marker() {
    let (dom, laid) = layout(
        r#"<ol><li style="list-style-type: none;">a</li><li>b</li></ol>"#,
        "body { margin: 0; }",
    );
    let mut lis = Vec::new();
    find_all_tags(&dom, dom.document(), "li", &mut lis);

    let first = find_laid_out(&laid, lis[0]).expect("li box not found");
    assert!(first.marker.is_none());
    let second = find_laid_out(&laid, lis[1]).expect("li box not found");
    assert_eq!(second.marker.as_ref().unwrap().runs[0].text, "2.");
}

#[test]
fn list_style_position_inside_wraps_the_marker_with_the_text_instead_of_a_gutter_box() {
    let (dom, laid) = layout(
        r#"<ul style="list-style-position: inside;"><li>hello</li></ul>"#,
        "body { margin: 0; }",
    );
    let li = find_tag(&dom, dom.document(), "li").expect("li not found");
    let li_box = find_laid_out(&laid, li).expect("li box not found");

    // `inside`はマーカーをテキストの一部として先頭行に織り込むため、
    // 別立てのマーカーボックスは持たない。
    assert!(li_box.marker.is_none());
    let LaidOutContent::Inline(lines) = &li_box.content else {
        panic!("expected inline content");
    };
    // 単語間の空白はどのランの`text`にも literal には含まれない(gapとして
    // 位置だけで表現される、既存の簡略化)ため、ラン自体の並びを確認する。
    let run_texts: Vec<&str> = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(run_texts, vec!["•", "hello"]);
}

#[test]
fn various_list_style_types_render_a_valid_pdf_end_to_end() {
    let html_src = r#"
        <ul><li style="list-style-type: circle;">a</li></ul>
        <ul><li style="list-style-type: square;">b</li></ul>
        <ol><li style="list-style-type: decimal-leading-zero;">c</li></ol>
        <ol><li style="list-style-type: lower-roman;">d</li></ol>
        <ol><li style="list-style-type: upper-roman;">e</li></ol>
        <ol><li style="list-style-type: lower-alpha;">f</li></ol>
        <ol><li style="list-style-type: upper-alpha;">g</li></ol>
    "#;
    let (page_count, _bytes) = build_pdf(html_src, "body { margin: 0; }");
    assert_eq!(page_count, 1);
}

#[test]
fn list_style_shorthand_applies_type_position_and_falls_back_from_image() {
    let (dom, laid) = layout(
        r#"<ul style="list-style: square inside url(does-not-exist.png);"><li>x</li></ul>"#,
        "body { margin: 0; }",
    );
    let li = find_tag(&dom, dom.document(), "li").expect("li not found");
    let li_box = find_laid_out(&laid, li).expect("li box not found");

    // `list-style-image`は常に`list-style-type`のテキストマーカーへ
    // フォールバックする。`inside`なのでspansに埋め込まれる。
    assert!(li_box.marker.is_none());
    let LaidOutContent::Inline(lines) = &li_box.content else {
        panic!("expected inline content");
    };
    let run_texts: Vec<&str> = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(run_texts, vec!["▪", "x"]);
}
