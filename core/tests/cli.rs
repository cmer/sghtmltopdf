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

// ---------------------------------------------------------------------------
// M12 Phase 1(T280〜T284): clap移行・stdin/stdout・exit code
// ---------------------------------------------------------------------------

#[test]
fn reads_html_from_stdin_when_the_input_is_a_dash() {
    use std::io::Write;
    use std::process::Stdio;

    let output = temp_output_path("stdin");
    let mut child = Command::new(BIN)
        .arg("-")
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(&output)
        .stdin(Stdio::piped())
        .spawn()
        .expect("failed to spawn sghtmltopdf");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"<html><body><p>from stdin</p></body></html>")
        .unwrap();
    let status = child.wait().expect("failed to wait for sghtmltopdf");
    assert!(status.success(), "CLI should accept HTML on stdin");

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert!(bytes.starts_with(b"%PDF-"));
    std::fs::remove_file(&output).ok();
}

#[test]
fn writes_the_pdf_to_stdout_when_the_output_is_a_dash() {
    let out = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg("-")
        .output()
        .expect("failed to run sghtmltopdf binary");

    assert!(out.status.success());
    assert!(
        out.stdout.starts_with(b"%PDF-"),
        "PDF bytes should go to stdout"
    );
    assert!(count_occurrences(&out.stdout, b"%%EOF") > 0);
    // 進捗メッセージはstdoutを汚さない(必ずstderrへ)。
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("標準出力"),
        "progress message should be written to stderr"
    );
}

#[test]
fn stdin_input_without_an_explicit_output_is_a_usage_error() {
    let out = Command::new(BIN)
        .arg("-")
        .arg("--font")
        .arg(FONT_PATH)
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn exit_codes_follow_the_documented_mapping() {
    // 1 = 使用法エラー(必須の--fontが無い)
    let usage = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(
        usage.status.code(),
        Some(1),
        "missing --font is a usage error"
    );

    // 1 = 使用法エラー(未知のオプション)
    let unknown = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("--no-such-option")
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(unknown.status.code(), Some(1));

    // 2 = 入力/リソースエラー(入力HTMLが存在しない)
    let input = Command::new(BIN)
        .arg(Path::new("/nonexistent/does-not-exist.html"))
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(temp_output_path("exit-code-input"))
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(input.status.code(), Some(2));
}

#[test]
fn version_and_help_exit_successfully() {
    for flag in ["--version", "--help"] {
        let out = Command::new(BIN)
            .arg(flag)
            .output()
            .expect("failed to run sghtmltopdf binary");
        assert!(out.status.success(), "{flag} should exit with 0");
        assert!(!out.stdout.is_empty(), "{flag} should print to stdout");
    }
}

#[test]
fn a_failing_run_leaves_no_output_file_behind() {
    // --fontにフォントではないファイル(HTML自身)を渡して失敗させる。
    // FileSinkは一時ファイルへ書いてからrenameするため、失敗しても
    // 出力先には何も残らない([0055]決定4)。
    let output = temp_output_path("no-leftover");
    let out = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(SAMPLE_HTML)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("failed to run sghtmltopdf binary");

    assert!(!out.status.success(), "loading a non-font file should fail");
    assert!(
        !output.exists(),
        "no partial PDF should be left at the output path"
    );

    // 一時ファイル(<output>.tmp-<pid>)も残っていないこと。
    let dir = output.parent().unwrap();
    let stem = output.file_name().unwrap().to_string_lossy().to_string();
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&stem) && name.contains(".tmp-")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files should be cleaned up, found: {leftovers:?}"
    );
}

#[test]
fn quiet_suppresses_the_success_message() {
    let output = temp_output_path("quiet");
    let out = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("-o")
        .arg(&output)
        .arg("--quiet")
        .output()
        .expect("failed to run sghtmltopdf binary");

    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "--quiet should suppress the success message, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&output).ok();
}

#[test]
fn base_url_directory_resolves_relative_assets_for_stdin_input() {
    use std::io::Write;
    use std::process::Stdio;

    // 標準入力から読むと相対解決の基準が無くなるため、--base-urlで与える。
    let dir = std::env::temp_dir().join(format!("sghtmltopdf-e2e-base-url-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(CJK_FONT_PATH, dir.join("cjk.ttc")).unwrap();
    let output = dir.join("out.pdf");

    let mut child = Command::new(BIN)
        .arg("-")
        .arg("--font")
        .arg(FONT_PATH)
        .arg("--base-url")
        .arg(&dir)
        .arg("-o")
        .arg(&output)
        .stdin(Stdio::piped())
        .spawn()
        .expect("failed to spawn sghtmltopdf");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            r#"<html><head><style>
                @font-face { font-family: "CJK Brand"; src: url("cjk.ttc"); }
                p { font-family: "CJK Brand"; }
            </style></head><body><p>日本語のテスト</p></body></html>"#
                .as_bytes(),
        )
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert_eq!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2"),
        2,
        "the @font-face font must be resolved relative to --base-url"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_bad_base_url_is_reported_as_an_input_error() {
    let out = Command::new(BIN)
        .arg(SAMPLE_HTML)
        .arg("--font")
        .arg(FONT_PATH)
        .arg("--base-url")
        .arg("/nonexistent/directory")
        .arg("-o")
        .arg(temp_output_path("bad-base-url"))
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn the_server_subcommand_reports_that_it_is_not_implemented_yet() {
    let out = Command::new(BIN)
        .arg("server")
        .output()
        .expect("failed to run sghtmltopdf binary");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("server"));
}
