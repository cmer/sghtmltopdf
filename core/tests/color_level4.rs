//! Color Level 4(`lab()`/`lch()`/`oklab()`/`oklch()`)のE2Eテスト。
//!
//! `box_sizing.rs`と同じ方針: 実際のパイプライン(HTMLパース→スタイルカスケード
//! →ページ分割→PDFエンコード)を通して回帰を検知する。

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

/// `/CreationDate`の値を伏せる。
///
/// PDFのInfo辞書には生成時刻が必ず入るので、別々に生成した2つのPDFを
/// そのまま比較すると、2回の生成が秒境界をまたいだときだけ落ちる。
/// 値は固定長(`D:YYYYMMDDHHMMSSZ`)なので、同じ長さで埋めればそれ以降の
/// バイト位置(相互参照テーブルのオフセット)はずれない。
fn mask_creation_date(bytes: &[u8]) -> Vec<u8> {
    const KEY: &[u8] = b"/CreationDate (";
    let mut out = bytes.to_vec();
    let Some(key_at) = out.windows(KEY.len()).position(|w| w == KEY) else {
        return out;
    };
    let value_at = key_at + KEY.len();
    let Some(value_len) = out[value_at..].iter().position(|&b| b == b')') else {
        return out;
    };
    out[value_at..value_at + value_len].fill(b'X');
    out
}

/// 生成時刻を除いて2つのPDFが同一であることを確かめる。
///
/// 食い違う場合は最初の位置と周辺だけを出す。数万バイトの配列を
/// `assert_eq!`に渡すと、差分ではなく両方の中身が丸ごと出力されて読めない。
fn assert_same_pdf(left: &[u8], right: &[u8]) {
    let (left, right) = (mask_creation_date(left), mask_creation_date(right));
    let first_diff = left
        .iter()
        .zip(right.iter())
        .position(|(a, b)| a != b)
        .or_else(|| (left.len() != right.len()).then_some(left.len().min(right.len())));
    let Some(at) = first_diff else {
        return;
    };
    let window = |bytes: &[u8]| {
        let from = at.saturating_sub(40);
        let to = (at + 40).min(bytes.len());
        String::from_utf8_lossy(&bytes[from..to]).to_string()
    };
    panic!(
        "PDFが{at}バイト目から食い違います({}バイト vs {}バイト)\n  left : {:?}\n  right: {:?}",
        left.len(),
        right.len(),
        window(&left),
        window(&right)
    );
}

// oklch(59.686% 0.15619 49.7694deg)はrgb(198, 93, 6)相当。色空間変換が
// パイプライン全体(スタイルカスケード→PDFエンコード)を通して正しく
// RgbaColorへ落ちることを、同じRGB値を直接指定した場合とのバイト列一致で確認する。
#[test]
fn oklch_background_color_matches_equivalent_rgb_byte_for_byte() {
    let oklch_bytes = build_pdf(".box { background-color: oklch(59.686% 0.15619 49.7694deg); }");
    let rgb_bytes = build_pdf(".box { background-color: rgb(198, 93, 6); }");
    assert_same_pdf(&oklch_bytes, &rgb_bytes);
}

/// 上の比較が生成時刻の違いを無視していること。
///
/// 秒境界をまたいで生成された2つのPDFを模して、日付の秒の桁だけを書き換える。
/// この扱いが無いと、2回の生成がたまたま別の秒に入ったときだけ落ちる。
#[test]
fn the_comparison_ignores_the_creation_timestamp() {
    const KEY: &[u8] = b"/CreationDate (";
    let bytes = build_pdf(".box { background-color: rgb(1, 2, 3); }");

    let mut later = bytes.clone();
    let value_at = later.windows(KEY.len()).position(|w| w == KEY).unwrap() + KEY.len();
    // `D:YYYYMMDDHHMMSSZ`の秒の下1桁。
    let seconds_ones = value_at + 15;
    later[seconds_ones] = if later[seconds_ones] == b'9' {
        b'0'
    } else {
        b'9'
    };

    assert_ne!(bytes, later, "前提: 日付だけが違うバイト列になっている");
    assert_same_pdf(&bytes, &later);
}
