//! テーブルセルの`rowspan`のE2Eテスト(M8 Phase 1 Table layout完全対応)。
//!
//! `table_caption.rs`/`table_vertical_align.rs`と同じ方針: 実際の
//! パイプラインを通して回帰を検知する。座標の詳細な検証は`layout_document`
//! (ページ分割前)の結果に対して行う。詳細設計は
//! [0021](../../docs/decisions/0021-table-layout-design.md)参照。

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
        LaidOutContent::Grid(grid) => grid
            .rows
            .iter()
            .flat_map(|row| &row.items)
            .find_map(|item| find_laid_out(item, target)),
        LaidOutContent::Table(table) => table
            .caption
            .as_deref()
            .and_then(|c| find_laid_out(c, target))
            .or_else(|| {
                table
                    .rows
                    .iter()
                    .flat_map(|row| &row.cells)
                    .find_map(|cell| find_laid_out(cell, target))
            }),
        LaidOutContent::Inline(_) | LaidOutContent::Image(_) => None,
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

#[test]
fn rowspan_cell_spans_two_rows_and_the_next_row_flows_around_it_end_to_end() {
    let html_src = r#"<table>
        <tr><td rowspan="2" style="height: 80px;">tall</td><td style="height: 10px;">a</td></tr>
        <tr><td style="height: 10px;">b</td></tr>
    </table>"#;
    let css = "body { margin: 0; }";

    let (dom, laid) = layout(html_src, css);
    let mut tds = Vec::new();
    find_all_tags(&dom, dom.document(), "td", &mut tds);
    assert_eq!(tds.len(), 3);

    let tall = find_laid_out(&laid, tds[0]).expect("tall cell not found");
    let a = find_laid_out(&laid, tds[1]).expect("cell a not found");
    let b = find_laid_out(&laid, tds[2]).expect("cell b not found");

    assert!(
        (tall.layout.margin_box_height() - 80.0).abs() < 0.5,
        "the rowspan cell should span the combined height of both rows: {}",
        tall.layout.margin_box_height()
    );
    // "b"は"tall"が占有する列(col0)を避けて"a"と同じ列(col1)に流れ、
    // "tall"の直下(y=40px)から始まるはず。
    assert!(
        (b.layout.border_box().x - a.layout.border_box().x).abs() < 0.5,
        "cell b should land in the same column as cell a, skipping the rowspan cell's column"
    );
    assert!(
        (b.layout.border_box().y - 40.0).abs() < 0.5,
        "cell b should start after row0's height(40px): {}",
        b.layout.border_box().y
    );

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
}

#[test]
fn table_without_rowspan_behaves_as_before_end_to_end() {
    let html_src = r#"<table><tr><td>a</td><td>b</td></tr></table>"#;
    let css = "body { margin: 0; }";

    let (dom, laid) = layout(html_src, css);
    let mut tds = Vec::new();
    find_all_tags(&dom, dom.document(), "td", &mut tds);
    let a = find_laid_out(&laid, tds[0]).expect("cell a not found");
    let b = find_laid_out(&laid, tds[1]).expect("cell b not found");

    assert_eq!(a.layout.border_box().y, 0.0);
    assert_eq!(b.layout.border_box().y, 0.0);
    assert!(
        (b.layout.border_box().x - (a.layout.border_box().x + a.layout.border_box().width)).abs()
            < 0.5,
        "cells without rowspan should still sit side by side with no gap"
    );
}
