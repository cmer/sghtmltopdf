//! Color Level 4(`lab()`/`lch()`/`oklab()`/`oklch()`)のE2Eテスト(M9 Phase 2)。
//!
//! `box_sizing.rs`と同じ方針: 実際のパイプライン(HTMLパース→スタイルカスケード
//! →ページ分割→PDFエンコード)を通して回帰を検知する。詳細設計は
//! [0029](../../docs/decisions/0029-color-level4-design.md)参照。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{paginate_document, PageSettings};
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

fn build_pdf(css: &str) -> Vec<u8> {
    let dom = html::parse(br#"<div class="box">color test</div>"#);
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

#[test]
fn lab_background_color_renders_a_valid_pdf_end_to_end() {
    let bytes = build_pdf(".box { background-color: lab(53.2408% 80.0925 67.2032); }");
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
}

#[test]
fn lch_background_color_renders_a_valid_pdf_end_to_end() {
    let bytes = build_pdf(".box { background-color: lch(53.2408% 104.5518 39.999deg); }");
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
}

#[test]
fn oklab_background_color_renders_a_valid_pdf_end_to_end() {
    let bytes = build_pdf(".box { background-color: oklab(62.8% 0.2249 0.1258); }");
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
}

// oklch(59.686% 0.15619 49.7694deg)はrgb(198, 93, 6)相当。色空間変換が
// パイプライン全体(スタイルカスケード→PDFエンコード)を通して正しく
// RgbaColorへ落ちることを、同じRGB値を直接指定した場合とのバイト列一致で確認する。
#[test]
fn oklch_background_color_matches_equivalent_rgb_byte_for_byte() {
    let oklch_bytes = build_pdf(".box { background-color: oklch(59.686% 0.15619 49.7694deg); }");
    let rgb_bytes = build_pdf(".box { background-color: rgb(198, 93, 6); }");
    assert_eq!(oklch_bytes, rgb_bytes);
}
