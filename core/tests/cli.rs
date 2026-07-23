//! T10: CLI結合・E2Eテスト。
//!
//! 実際にコンパイル済みバイナリを起動し、サンプルHTML(見出し+段落+
//! ページ分割が発生するだけの長さの繰り返しコンテンツ)を一括変換して、
//! 有効なPDFが生成されることを確認する。
//!
//! バイト単位のゴールデンPDF比較ではなく構造的なチェック(ページ数・
//! フォント埋め込みマーカーの有無)にとどめる。改ページパターン
//! (break-before/after/inside・orphans/widows)ごとの回帰検出は
//! `fragmentation.rs`(T16)が担当する。

use std::path::Path;
use std::process::Command;

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
const CJK_FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fonts/NotoSansCJK-Regular.ttc"
);
const SAMPLE_HTML: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.html");
const BIN: &str = env!("CARGO_BIN_EXE_sghtmltopdf");

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn temp_output_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sghtmltopdf-e2e-{}-{name}.pdf", std::process::id()))
}

#[test]
fn converts_sample_html_into_a_multi_page_pdf() {
    let output = temp_output_path("sample");

    let status = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to run sghtmltopdf binary");
    assert!(status.success(), "CLI should exit successfully");

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
    assert!(
        count_occurrences(&bytes, b"/Subtype /Type0") > 0,
        "font should be embedded"
    );
    assert!(count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0);

    let page_count = count_occurrences(&bytes, b"/MediaBox");
    assert!(
        page_count > 1,
        "sample.html has enough repeated content to force pagination, got {page_count} page(s)"
    );

    std::fs::remove_file(&output).ok();
}

#[test]
fn defaults_output_path_to_input_with_pdf_extension() {
    // 一時ディレクトリへ入力HTMLをコピーし、-oを省略して既定の出力先を確認する。
    let dir = std::env::temp_dir().join(format!("sghtmltopdf-e2e-default-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("input.html");
    std::fs::copy(SAMPLE_HTML, &input).unwrap();

    let status = Command::new(BIN)
        .arg(&input)
        .arg("--font")
        .arg(FONT_PATH)
        .status()
        .expect("failed to run sghtmltopdf binary");
    assert!(status.success());

    let expected_output = dir.join("input.pdf");
    assert!(
        expected_output.exists(),
        "default output path should be input path with .pdf extension"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn font_face_src_url_is_resolved_relative_to_the_html_file_and_embedded() {
    // HTMLファイルと同じディレクトリに置いたフォントファイルを、
    // `@font-face { src: url(...); }`の相対パスとして解決できることを確認する。
    // `--font`ではDejaVu Sans(CJKグリフを持たない)のみを渡し、CJKテキストは
    // `@font-face`経由で読み込んだフォントでのみ描画できるようにすることで、
    // 単に`--font`だけで埋め込まれたのではないことを検証する。
    let dir =
        std::env::temp_dir().join(format!("sghtmltopdf-e2e-font-face-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(CJK_FONT_PATH, dir.join("cjk.ttc")).unwrap();

    let input = dir.join("input.html");
    std::fs::write(
        &input,
        r#"<html><head><style>
            @font-face { font-family: "CJK Brand"; src: url("cjk.ttc"); }
            p { font-family: "CJK Brand"; }
        </style></head><body><p>日本語のテスト</p></body></html>"#,
    )
    .unwrap();

    let output = dir.join("output.pdf");
    let status = Command::new(BIN)
        .arg(&input)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to run sghtmltopdf binary");
    assert!(status.success(), "CLI should exit successfully");

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert_eq!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2"),
        2,
        "both the --font fallback and the @font-face font should be embedded"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fails_with_nonzero_exit_when_font_is_missing() {
    let output = temp_output_path("missing-font");

    let status = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to run sghtmltopdf binary");

    assert!(
        !status.success(),
        "CLI should fail when --font is not provided"
    );
    assert!(
        !output.exists(),
        "no output file should be created on failure"
    );
}

#[test]
fn fails_with_nonzero_exit_when_input_file_does_not_exist() {
    let output = temp_output_path("missing-input");

    let status = Command::new(BIN)
        .arg(Path::new("/nonexistent/does-not-exist.html"))
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to run sghtmltopdf binary");

    assert!(
        !status.success(),
        "CLI should fail when the input file does not exist"
    );
    assert!(!output.exists());
}
