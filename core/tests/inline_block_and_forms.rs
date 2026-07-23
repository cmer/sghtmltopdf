//! `display: inline-block`とフォーム要素の静的描画のE2Eテスト
//! (M10 カテゴリI、T249〜T254)。
//!
//! 設計は[0043](../../docs/decisions/0043-inline-block-and-form-controls-design.md)。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
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

/// 文書内の全ての行ボックスを出現順に集める。
fn all_lines(b: &LaidOutBox) -> Vec<LineBox> {
    fn walk(b: &LaidOutBox, out: &mut Vec<LineBox>) {
        match &b.content {
            LaidOutContent::Inline(lines) => out.extend(lines.iter().cloned()),
            LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
                for c in children {
                    walk(c, out);
                }
            }
            LaidOutContent::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        walk(cell, out);
                    }
                }
            }
            LaidOutContent::Image(_) => {}
        }
    }
    let mut out = Vec::new();
    walk(b, &mut out);
    out
}

fn find_tag(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == tag {
            return Some(id);
        }
    }
    dom.children(id).find_map(|c| find_tag(dom, c, tag))
}

fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
    if b.node == Some(target) {
        return Some(b);
    }
    match &b.content {
        LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
            children.iter().find_map(|c| find_laid_out(c, target))
        }
        _ => None,
    }
}

// ===== display: inline-block =====

#[test]
fn an_inline_block_sits_on_the_same_line_as_the_surrounding_text() {
    let (_, laid) = layout(
        r#"<p>before <span class="ib">box</span> after</p>"#,
        "body { margin: 0; } .ib { display: inline-block; width: 40px; height: 20px; }",
    );
    let lines = all_lines(&laid);
    assert_eq!(lines.len(), 1, "everything must fit on one line");
    assert_eq!(lines[0].atomics.len(), 1);
    // 箱の前後にテキストがある(=行の途中に置かれている)。
    let atomic = &lines[0].atomics[0];
    assert!(atomic.x_offset > 0.0, "the box should follow 'before'");
    assert_eq!(atomic.margin_box_width, 40.0);
}

#[test]
fn an_inline_block_grows_the_line_and_its_block() {
    let (dom, laid) = layout(
        r#"<p>text</p><p>text <span class="ib">box</span></p>"#,
        "body { margin: 0; } p { margin: 0; } .ib { display: inline-block; width: 40px; height: 60px; }",
    );
    let mut ps = Vec::new();
    fn collect(dom: &Dom, id: NodeId, out: &mut Vec<NodeId>) {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == "p" {
                out.push(id);
            }
        }
        for c in dom.children(id) {
            collect(dom, c, out);
        }
    }
    collect(&dom, dom.document(), &mut ps);

    let plain = find_laid_out(&laid, ps[0]).unwrap();
    let with_box = find_laid_out(&laid, ps[1]).unwrap();
    assert!(
        with_box.layout.content.height >= 60.0,
        "the paragraph must be at least as tall as the box, got {}",
        with_box.layout.content.height
    );
    assert!(with_box.layout.content.height > plain.layout.content.height);
    // 次の段落が重ならない(箱の高さが親のフローに反映されている)。
    assert!(with_box.layout.content.y >= plain.layout.content.y + plain.layout.content.height);
}

#[test]
fn a_line_containing_only_inline_blocks_still_takes_space() {
    // 回帰テスト: 行にテキストランが1つも無いと行ごと捨てられていた。
    let (dom, laid) = layout(
        r#"<p><span class="ib">a</span></p><p>after</p>"#,
        "body { margin: 0; } p { margin: 0; } .ib { display: inline-block; width: 30px; height: 30px; }",
    );
    let lines = all_lines(&laid);
    assert_eq!(lines.len(), 2, "the box-only line must exist");
    assert_eq!(lines[0].atomics.len(), 1);
    assert!(lines[0].rect.height >= 30.0, "got {}", lines[0].rect.height);

    let after = find_tag(&dom, dom.document(), "p").unwrap();
    let _ = after;
    assert!(
        lines[1].rect.y >= lines[0].rect.y + lines[0].rect.height,
        "the following text must not overlap the box"
    );
}

#[test]
fn inline_blocks_wrap_to_the_next_line_when_they_do_not_fit() {
    let (_, laid) = layout(
        r#"<p><span class="ib">a</span> <span class="ib">b</span> <span class="ib">c</span></p>"#,
        "body { margin: 0; } p { width: 150px; } \
         .ib { display: inline-block; width: 70px; height: 10px; }",
    );
    let lines = all_lines(&laid);
    assert!(lines.len() >= 2, "three 70px boxes cannot fit in 150px");
    assert!(lines.iter().all(|l| !l.atomics.is_empty()));
}

#[test]
fn an_inline_block_uses_its_content_width_when_width_is_auto() {
    let (_, laid) = layout(
        r#"<p><span class="ib">short</span></p>"#,
        "body { margin: 0; } .ib { display: inline-block; padding: 0 5px; }",
    );
    let lines = all_lines(&laid);
    let atomic = &lines[0].atomics[0];
    assert!(
        atomic.margin_box_width > 10.0 && atomic.margin_box_width < 200.0,
        "shrink-to-fit width should follow the content, got {}",
        atomic.margin_box_width
    );
}

#[test]
fn vertical_align_top_aligns_the_box_with_the_top_of_the_line() {
    let (_, laid) = layout(
        r#"<p><span class="tall">t</span><span class="short">s</span></p>"#,
        "body { margin: 0; } p { margin: 0; } \
         .tall { display: inline-block; width: 20px; height: 60px; } \
         .short { display: inline-block; width: 20px; height: 10px; vertical-align: top; }",
    );
    let line = &all_lines(&laid)[0];
    let short = line
        .atomics
        .iter()
        .find(|a| a.margin_box_height <= 20.0)
        .expect("short box");
    // 上端が行の上端に一致する。
    assert!(
        (short.content.layout.border_box().y - line.rect.y).abs() < 0.01,
        "expected {}, got {}",
        line.rect.y,
        short.content.layout.border_box().y
    );
}

// ===== フォーム要素 =====

#[test]
fn a_text_input_renders_its_value_inside_a_box() {
    let (dom, laid) = layout(
        r#"<p><input type="text" value="Taro"></p>"#,
        "body { margin: 0; }",
    );
    let line = &all_lines(&laid)[0];
    assert_eq!(line.atomics.len(), 1, "the input is an atomic box");

    let input = find_tag(&dom, dom.document(), "input").unwrap();
    let _ = input;
    let inner = &line.atomics[0].content;
    assert_eq!(inner.layout.border.top, 1.0, "the input has a border");
    let LaidOutContent::Inline(inner_lines) = &inner.content else {
        panic!("expected inline content inside the input");
    };
    let text: String = inner_lines[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, "Taro");
}

#[test]
fn a_text_input_falls_back_to_its_placeholder() {
    let (_, laid) = layout(
        r#"<p><input type="text" placeholder="your name"></p>"#,
        "body { margin: 0; }",
    );
    let inner = &all_lines(&laid)[0].atomics[0].content;
    let LaidOutContent::Inline(inner_lines) = &inner.content else {
        panic!("expected inline content");
    };
    let text: String = inner_lines[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    // `input`のUA規則は`white-space: pre`なので、値の空白がそのまま残る。
    assert_eq!(text, "your name");
}

#[test]
fn a_select_shows_the_selected_option() {
    let (_, laid) = layout(
        r#"<p><select><option>first</option><option selected>second</option></select></p>"#,
        "body { margin: 0; }",
    );
    let inner = &all_lines(&laid)[0].atomics[0].content;
    let LaidOutContent::Inline(inner_lines) = &inner.content else {
        panic!("expected inline content");
    };
    let text: String = inner_lines[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, "second");
}

#[test]
fn a_select_without_a_selected_option_shows_the_first_one() {
    let (_, laid) = layout(
        r#"<p><select><option>alpha</option><option>beta</option></select></p>"#,
        "body { margin: 0; }",
    );
    let inner = &all_lines(&laid)[0].atomics[0].content;
    let LaidOutContent::Inline(inner_lines) = &inner.content else {
        panic!("expected inline content");
    };
    let text: String = inner_lines[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, "alpha");
}

#[test]
fn a_submit_button_uses_a_default_label() {
    let (_, laid) = layout(r#"<p><input type="submit"></p>"#, "body { margin: 0; }");
    let inner = &all_lines(&laid)[0].atomics[0].content;
    let LaidOutContent::Inline(inner_lines) = &inner.content else {
        panic!("expected inline content");
    };
    let text: String = inner_lines[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, "Submit");
}

#[test]
fn a_checkbox_is_a_small_square_without_text() {
    let (_, laid) = layout(
        r#"<p><input type="checkbox" checked> label</p>"#,
        "body { margin: 0; }",
    );
    let line = &all_lines(&laid)[0];
    let checkbox = &line.atomics[0];
    assert!(
        checkbox.margin_box_width < 20.0 && checkbox.margin_box_height < 20.0,
        "got {}x{}",
        checkbox.margin_box_width,
        checkbox.margin_box_height
    );
    let LaidOutContent::Inline(inner_lines) = &checkbox.content.content else {
        panic!("expected inline content");
    };
    assert!(inner_lines.is_empty(), "a checkbox has no text of its own");
    // ラベルのテキストは通常のランとして同じ行に残る。
    let label: String = line.runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(label, "label");
}

#[test]
fn a_hidden_input_is_not_rendered() {
    let (_, laid) = layout(
        r#"<p><input type="hidden" value="secret">visible</p>"#,
        "body { margin: 0; }",
    );
    let line = &all_lines(&laid)[0];
    assert!(line.atomics.is_empty());
    let text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "visible");
}

#[test]
fn a_form_encodes_to_a_valid_pdf() {
    let dom = html::parse(
        br#"<form>
              <p>Name: <input type="text" value="Taro"></p>
              <p>Type: <select><option selected>Company</option></select></p>
              <p><input type="checkbox" checked> Yes <input type="radio"> No</p>
              <p><textarea>free text</textarea></p>
              <p><button>Send</button> <input type="submit"> <input value="x" disabled></p>
            </form>"#,
    );
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet("");
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.windows(5).any(|w| w == b"%%EOF"));
}

// ===== インラインの`<img>`(M11 Phase 2、T268、[0046]) =====

fn jpeg_data_uri() -> String {
    use base64::Engine;
    let jpeg = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient.jpg"
    ))
    .expect("fixture image");
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    format!("data:image/jpeg;base64,{b64}")
}

fn layout_with_images(html_src: &str, css: &str) -> (Dom, LaidOutBox) {
    use sghtmltopdf_core::layout::{build_box_tree, resolve_images};
    use sghtmltopdf_core::pdf::ImageAssetCache;
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let fonts = test_fonts();
    let mut tree = build_box_tree(&dom, &styles);
    let cache = ImageAssetCache::new(std::path::PathBuf::from("."), false);
    resolve_images(&mut tree, &dom, &cache);
    let laid = layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    );
    (dom, laid)
}

#[test]
fn an_inline_image_sits_on_the_same_line_as_the_text() {
    let html_src = format!(
        r#"<p>icon <img src="{}" width="40" height="30"> text</p>"#,
        jpeg_data_uri()
    );
    let (_, laid) = layout_with_images(&html_src, "body { margin: 0; }");
    let lines = all_lines(&laid);
    assert_eq!(lines.len(), 1, "the image must not force its own line");
    assert_eq!(
        lines[0].atomics.len(),
        1,
        "the image is an atomic inline box"
    );
    // 画像の前後にテキストがある。
    let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "icontext");
}

#[test]
fn an_inline_image_uses_its_attribute_size() {
    let html_src = format!(
        r#"<p><img src="{}" width="40" height="30"></p>"#,
        jpeg_data_uri()
    );
    let (_, laid) = layout_with_images(&html_src, "body { margin: 0; }");
    let atomic = &all_lines(&laid)[0].atomics[0];
    assert_eq!(atomic.margin_box_width, 40.0);
    assert_eq!(atomic.margin_box_height, 30.0);
    match &atomic.content.content {
        LaidOutContent::Image(Some(_)) => {}
        other => panic!("expected an embedded image, got {other:?}"),
    }
}

#[test]
fn a_block_image_still_takes_its_own_line() {
    // `display: block`を明示した`<img>`は従来どおりブロック置換要素。
    let html_src = format!(
        r#"<p>before</p><img src="{}" width="40" height="30" style="display: block;"><p>after</p>"#,
        jpeg_data_uri()
    );
    let (_, laid) = layout_with_images(&html_src, "body { margin: 0; }");
    // ブロック画像は行(atomics)に載らない。
    let lines = all_lines(&laid);
    assert!(
        lines.iter().all(|l| l.atomics.is_empty()),
        "a block image should not be an atomic inline box"
    );
}

#[test]
fn a_vertical_align_applies_to_an_inline_image() {
    let html_src = format!(
        r#"<p>x<img src="{}" width="20" height="40" style="vertical-align: top;"></p>"#,
        jpeg_data_uri()
    );
    let (_, laid) = layout_with_images(&html_src, "body { margin: 0; } p { margin: 0; }");
    let line = &all_lines(&laid)[0];
    let img = &line.atomics[0];
    // 上端揃え: 画像の上端が行の上端に一致する。
    assert!(
        (img.content.layout.border_box().y - line.rect.y).abs() < 0.01,
        "expected {}, got {}",
        line.rect.y,
        img.content.layout.border_box().y
    );
}

#[test]
fn an_inline_image_is_embedded_in_the_pdf() {
    // 画像の解決は`Engine`のパイプライン全体を通す(`paginate_document`は
    // 内部でbox treeを組み直すため、テスト側で`resolve_images`しても効かない)。
    use sghtmltopdf_core::engine::{Engine, EngineOptions, FontSpec, Mode};
    use sghtmltopdf_core::sink::MemorySink;

    let html_src = format!(
        r#"<html><body><p>logo <img src="{}" width="32" height="24"> here</p></body></html>"#,
        jpeg_data_uri()
    );
    let options = EngineOptions {
        mode: Mode::Batch,
        fonts: vec![FontSpec {
            path: std::path::PathBuf::from(FONT_PATH),
            index: 0,
        }],
        ..EngineOptions::default()
    };
    let mut engine = Engine::new(options, MemorySink::new());
    engine.feed(html_src.as_bytes()).unwrap();
    let bytes = engine.finish().unwrap();

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(
        bytes.windows(10).any(|w| w == b"/DCTDecode"),
        "the inline JPEG must be embedded as an image XObject"
    );
}
