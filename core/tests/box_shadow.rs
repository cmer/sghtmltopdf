//! `box-shadow`のE2Eテスト(M9 Phase 2)。
//!
//! `box_sizing.rs`と同じ方針: 実際のパイプライン(HTMLパース→スタイル
//! カスケード→ページ分割→PDFエンコード)を通して回帰を検知する。詳細設計は
//! [0032](../../docs/decisions/0032-box-shadow-design.md)、半透明描画の基盤は
//! [0031](../../docs/decisions/0031-fill-alpha-design.md)参照。

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

/// PDFバイト列中の全`stream`〜`endstream`区間を展開して連結したものを返す。
/// 各ストリームの`/Length N`をパースして正確に`N`バイトを切り出す
/// (`\nendstream`を素朴に探すだけの実装は、フォント埋め込みバイナリ中に
/// 偶然そのバイト列が出現すると誤って区切ってしまうため、`engine.rs`の
/// テストモジュール内の同名ヘルパーと同じ正確な実装を使う)。
fn decompressed_stream_bytes(pdf_bytes: &[u8]) -> Vec<u8> {
    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find_subslice(&pdf_bytes[i..], b"/Length ") {
        let len_start = i + pos + b"/Length ".len();
        let mut len_end = len_start;
        while len_end < pdf_bytes.len() && pdf_bytes[len_end].is_ascii_digit() {
            len_end += 1;
        }
        let Some(length) = std::str::from_utf8(&pdf_bytes[len_start..len_end])
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
        else {
            i = len_end.max(i + pos + 1);
            continue;
        };
        let Some(stream_rel) = find_subslice(&pdf_bytes[len_end..], b"stream\n") else {
            break;
        };
        let data_start = len_end + stream_rel + b"stream\n".len();
        let data_end = data_start + length;
        if data_end > pdf_bytes.len() {
            i = len_end;
            continue;
        }
        let raw = &pdf_bytes[data_start..data_end];

        let mut decoder = flate2::read::ZlibDecoder::new(raw);
        let mut decompressed = Vec::new();
        if std::io::Read::read_to_end(&mut decoder, &mut decompressed).is_ok() {
            out.extend_from_slice(&decompressed);
        } else {
            out.extend_from_slice(raw);
        }
        out.push(b'\n');

        i = data_end;
    }
    out
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

#[test]
fn box_shadow_adds_extra_fill_drawing_before_the_background_end_to_end() {
    let with_shadow = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; background-color: white; \
                box-shadow: 4px 4px 8px rgba(0, 0, 0, 0.5); }",
    );
    let without_shadow = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; background-color: white; }",
    );
    assert!(
        with_shadow.len() > without_shadow.len(),
        "box-shadow should add extra drawing operators to the content stream"
    );
}

#[test]
fn box_shadow_none_draws_nothing_extra_end_to_end() {
    let with_none = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; box-shadow: none; }",
    );
    let without_declaration = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } .box { width: 100px; height: 60px; }",
    );
    assert_eq!(with_none, without_declaration);
}

#[test]
fn box_shadow_inset_is_parsed_but_not_rendered_end_to_end() {
    // [0032]決定1: `inset`はパースするが描画は非対応(既知の簡略化)。
    let with_inset = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; box-shadow: inset 4px 4px 8px rgba(0,0,0,0.5); }",
    );
    let without_declaration = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } .box { width: 100px; height: 60px; }",
    );
    assert_eq!(with_inset, without_declaration);
}

#[test]
fn box_shadow_with_zero_blur_draws_exactly_one_rect_end_to_end() {
    let bytes = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; box-shadow: 4px 4px rgb(0, 0, 0); }",
    );
    let decompressed = decompressed_stream_bytes(&bytes);
    // ぼかし無し(blur-radius: 0)は同心矩形近似のループをスキップし、
    // コア矩形1枚だけを描画する([0032]決定3)。`rounded_rect_path`は
    // 角丸パス(`m`/`l`/`c`/`h`)を使うため、`close_path`+`fill_nonzero`の
    // 組み合わせ(`h\nf\n`)の出現回数で描画枚数を数えられる。div自身に
    // background-colorが無いため、この出現はbox-shadow由来の1枚のみのはず。
    assert_eq!(count_occurrences(&decompressed, b"h\nf\n"), 1);
}

#[test]
fn box_shadow_with_blur_draws_multiple_concentric_rects_end_to_end() {
    let bytes = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; box-shadow: 0 0 20px rgba(0, 0, 0, 0.5); }",
    );
    let decompressed = decompressed_stream_bytes(&bytes);
    // ぼかし近似は4段階のリング+コア矩形=5枚([0032]決定3)。
    assert_eq!(count_occurrences(&decompressed, b"h\nf\n"), 5);
}

#[test]
fn box_shadow_comma_separated_list_draws_each_shadow_end_to_end() {
    let bytes = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; \
                box-shadow: 2px 2px rgb(255,0,0), 4px 4px rgb(0,0,255); }",
    );
    let decompressed = decompressed_stream_bytes(&bytes);
    // 各シャドウがblur-radius: 0(コア矩形1枚)なので、2つ合わせて2枚。
    assert_eq!(count_occurrences(&decompressed, b"h\nf\n"), 2);
}

#[test]
fn box_shadow_and_border_radius_render_a_valid_pdf_end_to_end() {
    let bytes = build_pdf(
        r#"<div class="box">x</div>"#,
        "body { margin: 0; } \
         .box { width: 100px; height: 60px; border-radius: 12px; \
                box-shadow: 4px 4px 8px rgba(0, 0, 0, 0.4); }",
    );
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
}
