//! Flexbox(`display: flex`)のE2Eテスト(M9 Phase 3)。
//!
//! `box_sizing.rs`と同じ方針: 実際のパイプライン(HTMLパース→スタイル
//! カスケード→ページ分割→PDFエンコード)を通して回帰を検知する。詳細設計は
//! [0034](../../docs/decisions/0034-flexbox-design.md)参照。

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

fn build_pdf(html_src: &str, css: &str) -> Vec<u8> {
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let settings = PageSettings::default();

    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
    bytes
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
    match &b.content {
        LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
            children.iter().find_map(|c| find_laid_out(c, target))
        }
        LaidOutContent::Inline(_) | LaidOutContent::Table(_) | LaidOutContent::Image(_) => None,
    }
}

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

fn divs(dom: &Dom) -> Vec<NodeId> {
    let mut out = Vec::new();
    find_all_tags(dom, dom.document(), "div", &mut out);
    out
}

#[test]
fn flex_direction_row_places_items_side_by_side() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; width: 300px; } \
         .a, .b { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    let b = find_laid_out(&laid, d[2]).unwrap();

    assert_eq!(a.layout.border_box().x, 0.0);
    assert_eq!(b.layout.border_box().x, 50.0);
    assert_eq!(a.layout.border_box().y, b.layout.border_box().y);
}

#[test]
fn flex_direction_column_stacks_items_vertically() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; flex-direction: column; width: 300px; } \
         .a, .b { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    let b = find_laid_out(&laid, d[2]).unwrap();

    assert_eq!(a.layout.border_box().y, 0.0);
    assert_eq!(b.layout.border_box().y, 20.0);
    assert_eq!(a.layout.border_box().x, b.layout.border_box().x);
}

#[test]
fn justify_content_space_between_pushes_items_to_the_edges() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; justify-content: space-between; width: 300px; } \
         .a, .b { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    let b = find_laid_out(&laid, d[2]).unwrap();

    assert_eq!(a.layout.border_box().x, 0.0);
    assert_eq!(b.layout.border_box().x, 250.0);
}

#[test]
fn align_items_center_centers_items_on_the_cross_axis() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; align-items: center; width: 300px; height: 100px; } \
         .a { width: 50px; height: 20px; } \
         .b { width: 50px; height: 40px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    // コンテナ高さ100pxの中央(50px)を軸に、高さ20pxのアイテムが中央寄せされる
    // ので、上端は50 - 10 = 40pxになるはず。
    assert_eq!(a.layout.border_box().y, 40.0);
}

#[test]
fn flex_grow_distributes_remaining_space() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; width: 300px; } \
         .a { flex-grow: 1; height: 20px; } \
         .b { width: 100px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    // .bが100px固定、残り200pxを.aがflex-grow:1で全部受け取る。
    assert_eq!(a.layout.border_box().width, 200.0);
}

#[test]
fn flex_shrink_zero_prevents_an_item_from_shrinking_below_its_basis() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; width: 150px; } \
         .a { width: 100px; flex-shrink: 0; height: 20px; } \
         .b { width: 100px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    assert_eq!(a.layout.border_box().width, 100.0);
}

#[test]
fn gap_adds_space_between_items() {
    let (dom, laid) = layout(
        r#"<div class="container"><div class="a">a</div><div class="b">b</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; gap: 10px; width: 300px; } \
         .a, .b { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let b = find_laid_out(&laid, d[2]).unwrap();
    assert_eq!(b.layout.border_box().x, 60.0);
}

#[test]
fn a_flex_item_can_contain_ordinary_block_and_table_content() {
    let (dom, laid) = layout(
        r#"<div class="container">
             <div class="a"><p>text</p></div>
             <table class="b"><tr><td>cell</td></tr></table>
           </div>"#,
        "body { margin: 0; } \
         .container { display: flex; width: 300px; } \
         .a { width: 100px; } \
         .b { width: 100px; }",
    );
    let mut tables = Vec::new();
    find_all_tags(&dom, dom.document(), "table", &mut tables);
    let table_box = find_laid_out(&laid, tables[0]).unwrap();
    assert!(matches!(table_box.content, LaidOutContent::Table(_)));
}

#[test]
fn a_nested_flex_container_lays_out_inside_a_flex_item() {
    let (dom, laid) = layout(
        r#"<div class="outer">
             <div class="inner"><div class="x">x</div><div class="y">y</div></div>
           </div>"#,
        "body { margin: 0; } \
         .outer { display: flex; width: 300px; } \
         .inner { display: flex; width: 200px; } \
         .x, .y { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    // d[0]=outer, d[1]=inner, d[2]=x, d[3]=y
    let y = find_laid_out(&laid, d[3]).unwrap();
    assert_eq!(y.layout.border_box().x, 50.0);
}

#[test]
fn a_flex_container_is_treated_as_an_atomic_unit_across_page_breaks() {
    // ページ残り高さより本文全体が僅かに大きい状態を作り、flexコンテナが
    // 内部で分割されず丸ごと次ページへ送られることを確認する
    // ([0034]決定3、`display: table`と同じアトミック扱い)。
    let page_height = PageSettings::default().content_height();
    let filler_height = page_height - 30.0;
    let html =
        r#"<div class="filler">filler</div><div class="container"><div class="a">a</div></div>"#;
    let css = format!(
        "body {{ margin: 0; }} \
         .filler {{ height: {filler_height}px; }} \
         .container {{ display: flex; width: 300px; }} \
         .a {{ width: 50px; height: 60px; }}"
    );
    let dom = html::parse(html.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(&css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);

    assert_eq!(pages.len(), 2, "container should be pushed whole to page 2");

    let mut container_ids = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut container_ids);
    let container_node = container_ids[1]; // filler, container, a の順

    fn found_on_page(page: &sghtmltopdf_core::layout::Page, target: NodeId) -> Option<&LaidOutBox> {
        page.boxes.iter().find_map(|b| find_laid_out(b, target))
    }
    assert!(
        found_on_page(&pages[0], container_node).is_none(),
        "container must not appear (even partially) on page 1"
    );
    let on_page2 = found_on_page(&pages[1], container_node).expect("container should be on page 2");
    assert!(matches!(on_page2.content, LaidOutContent::Flex(_)));
}

#[test]
fn flexbox_renders_a_valid_pdf_end_to_end() {
    let bytes = build_pdf(
        r#"<div class="invoice-header">
             <div class="company">Acme Corp</div>
             <div class="date">2026-07-23</div>
           </div>"#,
        "body { margin: 0; } \
         .invoice-header { display: flex; justify-content: space-between; align-items: center; }",
    );
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
}
