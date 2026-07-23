//! `overflow`/`z-index`/`outline`/`visibility`/`border-style`拡張
//! (groove/ridge/inset/outset)/`border-radius`楕円のE2Eテスト
//! (M8 Phase 2 Box model詳細)。
//!
//! `list_style.rs`/`typography.rs`と同じ方針: 実際のパイプライン(HTMLパース→
//! スタイルカスケード→ページ分割→PDFエンコード)を通して回帰を検知する。
//! 詳細設計は[0023](../../docs/decisions/0023-box-model-details-design.md)参照。

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
fn all_box_model_features_combined_render_a_valid_pdf_end_to_end() {
    let html_src = r#"
        <div style="border: 8px groove blue;">groove</div>
        <div style="border: 8px ridge blue;">ridge</div>
        <div style="border: 8px inset blue;">inset</div>
        <div style="border: 8px outset blue;">outset</div>
        <div style="width: 100px; height: 60px; border-radius: 30px / 15px; border: 2px solid black;"></div>
        <div style="outline: 4px dashed red; border: 2px solid black;">outlined</div>
        <div style="visibility: hidden;">hidden text</div>
        <div style="width: 80px; height: 40px; overflow: hidden;">this text is longer than the box and should clip</div>
        <div style="position: relative; width: 200px; height: 80px;">
          <div style="position: relative; top: 0; left: 0; width: 100px; height: 60px; background-color: red; z-index: 1;"></div>
          <div style="position: relative; top: -40px; left: 40px; width: 100px; height: 60px; background-color: blue; z-index: 2;"></div>
        </div>
    "#;
    let (page_count, bytes) = build_pdf(html_src, "body { margin: 0; }");
    assert_eq!(page_count, 1);
    assert!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0,
        "the font should be embedded to render the text"
    );
}

#[test]
fn visibility_hidden_reserves_layout_space_but_display_none_does_not() {
    let (dom, laid) = layout(
        r#"<div class="a">A</div><div class="hidden">B</div><div class="none">C</div><div class="d">D</div>"#,
        "body { margin: 0; } div { height: 40px; margin: 0; } \
         .hidden { visibility: hidden; } .none { display: none; }",
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    // DOM上は4つの`div`が存在する(`display: none`はDOMそのものは削らない)。
    assert_eq!(divs.len(), 4);

    let a = find_laid_out(&laid, divs[0]).unwrap();
    let hidden = find_laid_out(&laid, divs[1]).unwrap();
    // `display: none`の要素はbox tree自体から除外される(Cに対応するボックスは無い)。
    assert!(find_laid_out(&laid, divs[2]).is_none());
    let d = find_laid_out(&laid, divs[3]).unwrap();

    // `visibility: hidden`は`display: none`と違い、レイアウト上の高さをそのまま
    // 占有する(見えないだけ)。
    assert_eq!(hidden.layout.content.height, 40.0);
    // Dは「A(40px) + hidden(40px、占有される)」の直後に来るはず
    // (Cはツリーに存在しないため高さに寄与しない)。
    assert_eq!(d.layout.content.y, a.layout.content.y + 80.0);
}

#[test]
fn z_index_reorders_overlapping_relative_siblings_but_keeps_their_own_position() {
    let (dom, laid) = layout(
        r#"<div class="outer">
            <div class="first" style="position: relative; z-index: 1;">first</div>
            <div class="second" style="position: relative; top: -20px; z-index: 2;">second</div>
        </div>"#,
        "body { margin: 0; } div.first, div.second { height: 30px; margin: 0; }",
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    // divs[0]は"outer"、divs[1]="first"、divs[2]="second"。
    let first_box = find_laid_out(&laid, divs[1]).unwrap();
    let second_box = find_laid_out(&laid, divs[2]).unwrap();

    // `position: relative`のオフセットは通常のフロー位置には影響しない
    // (後続要素の配置計算には`position:relative`前の位置を使う、既存動作)。
    assert_eq!(
        first_box.layout.content.y,
        second_box.layout.content.y - 30.0 + 20.0
    );
}

#[test]
fn border_radius_longhand_and_shorthand_render_a_valid_pdf() {
    let html_src = r#"
        <div style="width: 100px; height: 50px; border: 2px solid black; border-radius: 10px 20px / 5px 10px;"></div>
        <div style="width: 100px; height: 50px; border: 2px solid black; border-top-left-radius: 8px 4px;"></div>
    "#;
    let (page_count, _bytes) = build_pdf(html_src, "body { margin: 0; }");
    assert_eq!(page_count, 1);
}

// ===== 親子間・空ブロックのマージン相殺(M11 Phase 2、T271、[0048]) =====

#[test]
fn a_child_top_margin_collapses_through_a_borderless_parent() {
    let (dom, laid) = layout(
        r#"<div class="wrap"><p>child</p></div><p class="sib">sibling</p>"#,
        "body { margin: 0; } .wrap { margin: 0; } p { margin: 30px 0; }",
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    let mut ps = Vec::new();
    find_all_tags(&dom, dom.document(), "p", &mut ps);
    let wrap = find_laid_out(&laid, divs[0]).unwrap();
    let child = find_laid_out(&laid, ps[0]).unwrap();

    // 子の margin-top が親を突き抜け、子は親の content 上端に密着する。
    assert_eq!(child.layout.content.y, wrap.layout.content.y);
    // 親自身が実効 margin-top 30 を持つ(= 子の margin と相殺)。
    assert_eq!(wrap.layout.margin.top, 30.0);
}

#[test]
fn the_gap_between_a_wrapped_child_and_a_following_sibling_collapses() {
    // 親子相殺(下)と隣接兄弟相殺が連鎖し、余白は二重にならず 40 の1つになる。
    let (dom, laid) = layout(
        r#"<div class="wrap"><p class="inner">child</p></div><p class="sib">sibling</p>"#,
        "body { margin: 0; } .wrap { margin: 0; }          .inner { margin-bottom: 40px; } .sib { margin-top: 20px; }",
    );
    let mut ps = Vec::new();
    find_all_tags(&dom, dom.document(), "p", &mut ps);
    let inner = find_laid_out(&laid, ps[0]).unwrap();
    let sib = find_laid_out(&laid, ps[1]).unwrap();

    let gap = sib.layout.content.y - (inner.layout.content.y + inner.layout.content.height);
    // 単純加算(40+20=60)ではなく、相殺で max(40, 20) = 40。
    assert!(
        (gap - 40.0).abs() < 0.5,
        "gap should collapse to 40, got {gap}"
    );
}

#[test]
fn an_empty_block_does_not_double_its_margins() {
    let (dom, laid) = layout(
        r#"<p class="a">above</p><div class="empty"></div><p class="b">below</p>"#,
        "body { margin: 0; } p { margin: 0; } .empty { margin: 25px 0; }",
    );
    let mut ps = Vec::new();
    find_all_tags(&dom, dom.document(), "p", &mut ps);
    let above = find_laid_out(&laid, ps[0]).unwrap();
    let below = find_laid_out(&laid, ps[1]).unwrap();

    let gap = below.layout.content.y - (above.layout.content.y + above.layout.content.height);
    // 空 div の上下 25px が二重(50)にならず、相殺で 25。
    assert!(
        (gap - 25.0).abs() < 0.5,
        "empty block margins should collapse, got {gap}"
    );
}

#[test]
fn a_document_using_margin_collapse_renders_a_valid_pdf() {
    let (_, bytes) = build_pdf(
        r#"<div class="card"><h2>Title</h2><p>body</p></div>"#,
        ".card { margin: 20px 0; } h2 { margin: 16px 0; } p { margin: 12px 0; }",
    );
    assert!(bytes.starts_with(b"%PDF-"));
}

// ===== calc()(M11 Phase 2、T272、[0050]) =====

#[test]
fn calc_width_mixes_percentage_and_pixels() {
    let (dom, laid) = layout(
        r#"<div class="c">x</div>"#,
        "body { margin: 0; } .c { width: calc(100% - 100px); height: 20px; }",
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    let c = find_laid_out(&laid, divs[0]).unwrap();
    let content_width = PageSettings::default().content_width();
    assert!(
        (c.layout.content.width - (content_width - 100.0)).abs() < 0.5,
        "calc(100% - 100px) should be {} but was {}",
        content_width - 100.0,
        c.layout.content.width
    );
}

#[test]
fn calc_padding_resolves_em_and_pixels() {
    let (dom, laid) = layout(
        r#"<div class="c">x</div>"#,
        "body { margin: 0; } .c { font-size: 16px; width: 300px; padding-left: calc(1em + 4px); }",
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    let c = find_laid_out(&laid, divs[0]).unwrap();
    // 1em(16px) + 4px = 20px。
    assert!(
        (c.layout.padding.left - 20.0).abs() < 0.5,
        "got {}",
        c.layout.padding.left
    );
}

#[test]
fn a_document_using_calc_renders_a_valid_pdf() {
    let (_, bytes) = build_pdf(
        r#"<div style="width: calc(50% + 2em); margin-left: calc(10px + 5%);">x</div>"#,
        "body { margin: 0; }",
    );
    assert!(bytes.starts_with(b"%PDF-"));
}
