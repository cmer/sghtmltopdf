//! Flexbox(`display: flex`)のE2Eテスト。
//!
//! `box_sizing.rs`と同じ方針: 実際のパイプライン(HTMLパース→スタイル
//! カスケード→ページ分割→PDFエンコード)を通して回帰を検知する。

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
        LaidOutContent::Grid(grid) => grid
            .rows
            .iter()
            .flat_map(|row| &row.items)
            .find_map(|item| find_laid_out(item, target)),
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
fn a_flex_item_with_padding_grows_to_fit_its_wrapped_text() {
    // taffyがmeasureへ渡す既知サイズはborder-box基準なので、padding分を引かずに
    // 内容を採寸すると実際より広い幅で行分割してしまい、アイテムの高さが
    // 足りず内容がはみ出す。paddingの有無で内容の高さが変わらないことで、
    // content-box幅で採寸できていることを確かめる。
    let text = "wrap this text onto two lines";
    let (dom, laid) = layout(
        &format!(
            r#"<div class="container">
                 <div class="plain">{text}</div>
                 <div class="padded">{text}</div>
               </div>"#
        ),
        // stretchだと両方とも行のcross sizeまで伸びて差が消えるため、
        // 自然な高さを見るためにflex-startにする。
        "body { margin: 0; } \
         .container { display: flex; align-items: flex-start; width: 400px; } \
         .plain { flex: 0 0 100px; } \
         .padded { flex: 0 0 100px; padding: 10px; }",
    );
    let d = divs(&dom);
    let plain = find_laid_out(&laid, d[1]).unwrap();
    let padded = find_laid_out(&laid, d[2]).unwrap();

    // 同じ内容幅(100px)なので、paddingぶんだけ高い箱になるのが正しい。
    assert_eq!(
        padded.layout.border_box().height,
        plain.layout.border_box().height + 20.0
    );
    // 前提: この文章は幅100pxで複数行に折り返す(1行に収まると回帰を検知
    // できないため、行送りより高いことで確認する)。
    assert!(plain.layout.border_box().height > 20.0);
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
    // (`display: table`と同じアトミック扱い)。
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

// ===== 裸のテキストから作られる無名flexアイテム =====

/// レイアウト済みツリーの行テキストを出現順に連結して返す。
fn laid_out_text(b: &LaidOutBox) -> String {
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
                for line in lines {
                    for run in &line.runs {
                        out.push_str(&run.text);
                    }
                }
            }
            LaidOutContent::Table(_) | LaidOutContent::Image(_) => {}
        }
    }
    let mut out = String::new();
    walk(b, &mut out);
    out
}

/// 無名ボックス(`node`が`None`)である最初のflexアイテムを返す。
fn first_anonymous_flex_item(b: &LaidOutBox) -> Option<&LaidOutBox> {
    match &b.content {
        LaidOutContent::Flex(children) => children
            .iter()
            .find(|item| item.node.is_none())
            .or_else(|| children.iter().find_map(first_anonymous_flex_item)),
        LaidOutContent::Blocks(children) => children.iter().find_map(first_anonymous_flex_item),
        LaidOutContent::Grid(grid) => grid
            .rows
            .iter()
            .flat_map(|row| &row.items)
            .find_map(first_anonymous_flex_item),
        _ => None,
    }
}

#[test]
fn bare_text_in_a_flex_container_is_rendered() {
    // 回帰テスト: 要素で包まれていないテキストがflexアイテムにならず、
    // まるごと出力から消えていた(`<div style="display:flex">印</div>`のような
    // 中身が空の枠だけになる)。
    let (_, laid) = layout(
        r#"<div class="container">bare</div>"#,
        "body { margin: 0; } .container { display: flex; width: 200px; }",
    );

    let text = laid_out_text(&laid);
    assert!(
        text.contains("bare"),
        "the bare text should be rendered, got {text:?}"
    );

    // PDFまで通ることも確認する。
    build_pdf(
        r#"<div class="container">bare</div>"#,
        "body { margin: 0; } .container { display: flex; width: 200px; }",
    );
}

#[test]
fn bare_text_is_positioned_by_the_flex_alignment_properties() {
    // 無名アイテムも通常のflexアイテムと同じく整列の対象になる。
    let (_, laid) = layout(
        r#"<div class="container">x</div>"#,
        "body { margin: 0; } \
         .container { display: flex; justify-content: flex-end; width: 200px; }",
    );

    let item = first_anonymous_flex_item(&laid).expect("expected an anonymous flex item");
    assert!(
        item.layout.border_box().x > 0.0,
        "an end-aligned item should not sit at the container's left edge"
    );
}

#[test]
fn whitespace_between_flex_items_does_not_become_an_item() {
    // 要素の間の改行・インデントから無名アイテムを作ってしまうと、
    // 幅ゼロのアイテムが混ざって間隔が狂う。
    let (dom, laid) = layout(
        "<div class=\"container\">\n  <div class=\"a\">a</div>\n  <div class=\"b\">b</div>\n</div>",
        "body { margin: 0; } \
         .container { display: flex; width: 300px; } \
         .a, .b { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let a = find_laid_out(&laid, d[1]).unwrap();
    let b = find_laid_out(&laid, d[2]).unwrap();

    assert_eq!(a.layout.border_box().x, 0.0);
    assert_eq!(b.layout.border_box().x, 50.0);
    assert!(first_anonymous_flex_item(&laid).is_none());
}

#[test]
fn a_text_run_and_an_element_become_separate_items() {
    let (dom, laid) = layout(
        r#"<div class="container">left<div class="e">e</div></div>"#,
        "body { margin: 0; } \
         .container { display: flex; width: 300px; } \
         .e { width: 50px; height: 20px; }",
    );
    let d = divs(&dom);
    let element_item = find_laid_out(&laid, d[1]).unwrap();
    let anonymous = first_anonymous_flex_item(&laid).expect("expected an anonymous flex item");

    assert_eq!(anonymous.layout.border_box().x, 0.0);
    assert!(
        element_item.layout.border_box().x > 0.0,
        "the element item should follow the anonymous text item"
    );
    assert_eq!(laid_out_text(anonymous), "left");
}

/// flexアイテムの中身がさらにflexコンテナの場合。自然幅が0として測られて
/// いた頃は、内側のコンテナごと幅0に潰れていた(grid-in-grid・flex-in-grid・
/// grid-in-flexも同じ経路)。
#[test]
fn a_flex_inside_a_flex_item_does_not_collapse_to_zero() {
    let (dom, laid) = layout(
        r#"<div class="outer"><div class="item"><div class="a">alpha</div><div class="b">beta</div></div></div>"#,
        "body { margin: 0; } \
         .outer { display: flex; width: 400px; } \
         .item { display: flex; gap: 10px; }",
    );
    let d = divs(&dom);
    let item = find_laid_out(&laid, d[1]).unwrap();
    let a = find_laid_out(&laid, d[2]).unwrap();
    let b = find_laid_out(&laid, d[3]).unwrap();

    assert!(
        item.layout.content.width > 0.0,
        "内側のflexコンテナが潰れてはならない: {:?}",
        item.layout.content
    );
    assert!(a.layout.content.width > 0.0 && b.layout.content.width > 0.0);
    // 主軸が横なので、内側の自然幅は2つのアイテム+gapの合計になる。
    assert!(
        item.layout.content.width >= a.layout.content.width + b.layout.content.width,
        "a={:?} b={:?} item={:?}",
        a.layout.content,
        b.layout.content,
        item.layout.content
    );
}

// ===== 採寸した自然幅を丸めない =====

/// 与えられたノードのレイアウト済み行数を返す。
fn line_count(b: &LaidOutBox) -> usize {
    match &b.content {
        LaidOutContent::Inline(lines) => lines.len(),
        _ => panic!("インラインの内容を持つ箱ではない"),
    }
}

#[test]
fn a_flex_item_is_not_rounded_below_the_width_its_text_needs() {
    // 回帰テスト: taffyが既定で最終レイアウトを整数へ丸めるため、採寸で得た
    // 自然幅が切り捨てられ、内容より狭いアイテムになって収まるはずの行が
    // 折り返されていた。丸めの向きは端数次第なので、同じ書式の文字列でも
    // 特定の内容だけ折り返す、という形で表面化する。
    //
    // 下の2つはDejaVuSans 14pxで自然幅146.16pxと129.98px。丸めると前者だけが
    // 146へ切り捨てられて0.16px足りなくなり、`EUR`が2行目へ落ちていた。
    for value in ["1 USD = 0.9143 EUR", "1 USD 0.9143 EUR"] {
        let (dom, laid) = layout(
            &format!(
                r#"<div class="row"><span class="k">Exchange rate</span><span class="v">{value}</span></div>"#
            ),
            "body { margin: 0; font-size: 14px; } \
             .row { display: flex; justify-content: space-between; gap: 24px; } \
             .k { white-space: nowrap; } \
             .v { text-align: right; }",
        );
        let mut spans = Vec::new();
        find_all_tags(&dom, dom.document(), "span", &mut spans);
        let v = find_laid_out(&laid, spans[1]).unwrap();
        assert_eq!(
            line_count(v),
            1,
            "行に余裕があるので折り返してはならない: {value:?} width={:?}",
            v.layout.content
        );
    }
}

#[test]
fn flex_item_widths_keep_their_fractional_part() {
    // 上のテストの土台。アイテムの幅が整数へ丸められていないことを直接見る
    // (丸めが復活すると、端数が消えることで先に気付ける)。
    let (dom, laid) = layout(
        r#"<div class="row"><span class="v">1 USD = 0.9143 EUR</span></div>"#,
        "body { margin: 0; font-size: 14px; } \
         .row { display: flex; }",
    );
    let mut spans = Vec::new();
    find_all_tags(&dom, dom.document(), "span", &mut spans);
    let v = find_laid_out(&laid, spans[0]).unwrap();
    assert!(
        v.layout.content.width.fract() != 0.0,
        "自然幅の端数が失われている: {:?}",
        v.layout.content
    );
}
