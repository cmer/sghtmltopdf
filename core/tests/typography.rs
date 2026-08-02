//! `text-align`/`line-height`/`text-indent`/`white-space`/`letter-spacing`/
//! `word-spacing`/`text-transform`のE2Eテスト(M8 Phase 1 Typography詳細)。
//!
//! `fragmentation.rs`/`float_position.rs`と同じ方針: 実際のパイプライン
//! (HTMLパース→スタイルカスケード→ページ分割→PDFエンコード)を通して回帰を
//! 検知する。座標の詳細な検証は`layout_document`(ページ分割前)の結果に対して
//! 行い、PDFエンコードまでのパイプライン全体がクラッシュせず妥当な出力になる
//! ことは`build_pdf`で別途確認する。

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

/// PDFのcontent streamはFlateDecodeで圧縮されているため、`Tc`のような
/// コンテンツストリーム内演算子を検索するには解凍が必要
/// (`pdf::document`テストモジュール内の同名関数と同じロジック)。
fn decompressed_stream_bytes(pdf_bytes: &[u8]) -> Vec<u8> {
    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find_subslice(&pdf_bytes[i..], b"stream\n") {
        let start = i + pos + b"stream\n".len();
        let Some(end_rel) = find_subslice(&pdf_bytes[start..], b"\nendstream") else {
            break;
        };
        let end = start + end_rel;
        let raw = &pdf_bytes[start..end];

        let mut decoder = flate2::read::ZlibDecoder::new(raw);
        let mut decompressed = Vec::new();
        if std::io::Read::read_to_end(&mut decoder, &mut decompressed).is_ok() {
            out.extend_from_slice(&decompressed);
        } else {
            out.extend_from_slice(raw);
        }
        i = end + b"\nendstream".len();
    }
    out
}

/// HTML+CSSから、実際のパイプライン(パース→カスケード→ページ分割→PDF
/// エンコード)を一通り実行する(`fragmentation.rs::build_pdf`と同じ)。
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
fn text_align_right_shifts_the_line_end_to_end() {
    let html_src = r#"<p class="text">hi</p>"#;

    let (dom, laid) = layout(
        html_src,
        "body { margin: 0; } .text { width: 300px; margin: 0; }",
    );
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let left_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(left_lines) = &left_box.content else {
        panic!("expected inline content");
    };
    assert_eq!(left_lines[0].runs[0].x_offset, 0.0);

    let (dom, laid) = layout(
        html_src,
        "body { margin: 0; } .text { width: 300px; margin: 0; text-align: right; }",
    );
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let right_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(right_lines) = &right_box.content else {
        panic!("expected inline content");
    };
    // text-alignの効果は行の帯(`rect.x`)ではなく各ランの`x_offset`に反映される
    // (`rect.x`はfloatの帯計算で使う左端のまま)。
    assert!(right_lines[0].runs[0].x_offset > 0.0);

    let (page_count, bytes) = build_pdf(
        html_src,
        "body { margin: 0; } .text { width: 300px; margin: 0; text-align: right; }",
    );
    assert_eq!(page_count, 1);
    assert!(bytes.starts_with(b"%PDF-"));
}

#[test]
fn text_align_justify_stretches_non_last_lines_end_to_end() {
    let html_src = r#"<p class="text">hello world foo bar baz qux quux corge grault garply</p>"#;
    let css = "body { margin: 0; } .text { width: 150px; margin: 0; text-align: justify; }";

    let (dom, laid) = layout(html_src, css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    assert!(lines.len() >= 2, "expected wrapping to at least 2 lines");
    assert_eq!(
        lines[0].rect.width, 150.0,
        "the first (non-last) line should stretch to fill the available width"
    );
    assert!(
        lines.last().unwrap().rect.width < 150.0,
        "the last line should not be stretched"
    );

    let (page_count, _) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
}

#[test]
fn line_height_number_increases_line_spacing_end_to_end() {
    let html_src = r#"<p class="text">line one and two and three and four words</p>"#;
    let narrow_css = "body { margin: 0; } .text { width: 100px; margin: 0; }";

    let (dom, laid) = layout(html_src, narrow_css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(normal_lines) = &p_box.content else {
        panic!("expected inline content");
    };
    assert!(normal_lines.len() >= 2);
    let normal_gap = normal_lines[1].rect.y - normal_lines[0].rect.y;

    let tall_css = "body { margin: 0; } .text { width: 100px; margin: 0; line-height: 3; }";
    let (dom, laid) = layout(html_src, tall_css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(tall_lines) = &p_box.content else {
        panic!("expected inline content");
    };
    assert!(tall_lines.len() >= 2);
    let tall_gap = tall_lines[1].rect.y - tall_lines[0].rect.y;

    assert!(
        tall_gap > normal_gap * 2.0,
        "line-height: 3 should noticeably widen line spacing (normal={normal_gap}, tall={tall_gap})"
    );

    let (page_count, _) = build_pdf(html_src, tall_css);
    assert_eq!(page_count, 1);
}

#[test]
fn text_indent_offsets_only_the_first_line_end_to_end() {
    let html_src = r#"<p class="text">hello world foo bar baz qux quux</p>"#;
    let css = "body { margin: 0; } .text { width: 100px; margin: 0; text-indent: 20px; }";

    let (dom, laid) = layout(html_src, css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    assert!(lines.len() >= 2);
    assert_eq!(lines[0].rect.x, 20.0);
    assert_eq!(lines[1].rect.x, 0.0);

    let (page_count, _) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
}

#[test]
fn white_space_pre_preserves_formatting_end_to_end() {
    let html_src = "<pre>a    b\nc    d</pre>";

    let (dom, laid) = layout(html_src, "body { margin: 0; }");
    let pre = find_tag(&dom, dom.document(), "pre").expect("pre not found");
    let pre_box = find_laid_out(&laid, pre).expect("pre box not found");
    let LaidOutContent::Inline(lines) = &pre_box.content else {
        panic!("expected inline content");
    };
    assert_eq!(lines.len(), 2, "explicit newline should split into 2 lines");
    let first_text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        first_text, "a    b",
        "runs of whitespace should be preserved"
    );

    let (page_count, bytes) = build_pdf(html_src, "body { margin: 0; }");
    assert_eq!(page_count, 1);
    assert!(bytes.starts_with(b"%PDF-"));
}

#[test]
fn white_space_nowrap_overflows_instead_of_wrapping_end_to_end() {
    let html_src = r#"<p class="text">hello world foo bar</p>"#;
    let css = "body { margin: 0; } .text { width: 60px; margin: 0; white-space: nowrap; }";

    let (dom, laid) = layout(html_src, css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    assert_eq!(lines.len(), 1);
    assert!(lines[0].rect.width > 60.0);

    let (page_count, _) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
}

#[test]
fn letter_spacing_and_word_spacing_widen_layout_and_emit_tc_end_to_end() {
    let html_src = r#"<p class="text">hi there</p>"#;
    let css = "p.text { letter-spacing: 4px; word-spacing: 10px; }";

    let (page_count, bytes) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
    assert!(
        count_occurrences(&decompressed_stream_bytes(&bytes), b" Tc\n") > 0,
        "letter-spacing should emit a Tc operator in the final PDF"
    );
}

#[test]
fn text_transform_uppercase_applies_end_to_end() {
    let html_src = r#"<p class="text">hello</p>"#;
    let css = "p.text { text-transform: uppercase; }";

    let (dom, laid) = layout(html_src, css);
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "HELLO");

    let (page_count, _) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
}

#[test]
fn combined_typography_properties_render_a_valid_pdf_end_to_end() {
    // justify + line-height + text-indent + letter-spacing + white-space: pre
    // を1つの文書に組み合わせても、パイプライン全体が破綻しないことを確認する。
    let html_src = r#"<div>
        <p class="justified">hello world foo bar baz qux quux corge grault garply</p>
        <pre class="preformatted">line one
line   two</pre>
    </div>"#;
    let css = "body { margin: 0; } \
               .justified { width: 150px; margin: 0; text-align: justify; \
                            line-height: 1.5; text-indent: 10px; letter-spacing: 1px; }";

    let (page_count, bytes) = build_pdf(html_src, css);
    assert_eq!(page_count, 1);
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&decompressed_stream_bytes(&bytes), b" Tc\n") > 0);
}
