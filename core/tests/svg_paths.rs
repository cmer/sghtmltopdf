//! SVG参照のパス/URL解決のE2Eテスト。
//!
//! SVGはラスタ画像と同じ`img::fetch`を通るので、封じ込め(基準ディレクトリ・
//! `--allow`・`--disable-local-file-access`)とリモート取得の可否は共通の
//! はず。「共通のはず」を実際に確かめておかないと、SVGだけ別経路で読めて
//! しまっていても気付けない。読み出しに関わる部分なので、通る場合だけでなく
//! **拒否される場合**を同じ密度で見る。
//!
//! フォーマットの判定はバイト列で行う(拡張子や宣言mime typeは見ない)ので、
//! そこも併せて確認する。

#![cfg(feature = "svg")]

use std::path::PathBuf;
use std::process::Command;

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
const PNG_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/images/spike_opaque.png"
);
const BIN: &str = env!("CARGO_BIN_EXE_sghtmltopdf");

/// 20x10の最小SVG。青い矩形1つ。
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
  <rect width="20" height="10" fill="#0000ff"/>
</svg>"##;

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// SVGがベクタとして埋め込まれたか(Form XObjectになっているか)。
fn embedded_as_vector(pdf: &[u8]) -> bool {
    count_occurrences(pdf, b"/Subtype /Form") > 0
}

/// ラスタ画像として埋め込まれたか。
fn embedded_as_raster(pdf: &[u8]) -> bool {
    count_occurrences(pdf, b"/Subtype /Image") > 0
}

/// テスト1件分の作業ディレクトリ。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("sghtmltopdf-svgpath-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        self.write_bytes(relative, contents.as_bytes())
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.join(relative)
    }

    /// `html`(このディレクトリ配下の相対パス)をPDFへ変換する。
    fn convert(&self, html: &str, extra: &[&str]) -> Outcome {
        let output = self.dir.join("out.pdf");
        let result = Command::new(BIN)
            .arg(self.dir.join(html))
            .args(["--font", FONT_PATH])
            .args(extra)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("failed to run the sghtmltopdf binary");
        Outcome {
            success: result.status.success(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            pdf: std::fs::read(&output).unwrap_or_default(),
        }
    }

    /// 拒否された理由の文言。`--load-media-error-handling abort`にすると
    /// 取得できなかった理由がそのままエラーとして出るので、それを読む
    /// (既定の`ignore`は黙って続けるため理由が見えない)。
    fn refusal_reason(&self, html: &str, extra: &[&str]) -> String {
        let mut args = extra.to_vec();
        args.extend(["--load-media-error-handling", "abort"]);
        let outcome = self.convert(html, &args);
        assert!(
            !outcome.success,
            "abort should fail the conversion for a refused reference, stderr: {}",
            outcome.stderr
        );
        outcome.stderr
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

struct Outcome {
    success: bool,
    stderr: String,
    pdf: Vec<u8>,
}

impl Outcome {
    /// 変換は成功し、SVGがベクタとして入っていること。
    fn assert_rendered(&self) {
        assert!(
            self.success,
            "the conversion should succeed, stderr: {}",
            self.stderr
        );
        assert!(self.pdf.starts_with(b"%PDF-"));
        assert!(
            embedded_as_vector(&self.pdf),
            "the SVG should be embedded as a form XObject, stderr: {}",
            self.stderr
        );
    }

    /// 変換自体は成功するが、SVGは読まれず描画もされないこと。
    ///
    /// 取得失敗の既定は`--load-media-error-handling ignore`で、ラスタ画像と
    /// 同じく黙って空の置換要素になる(文書全体は止めない)。理由まで確かめたい
    /// ときは[`Fixture::refusal_reason`]を使う。
    fn assert_refused(&self) {
        assert!(
            self.success,
            "a refused reference should not fail the whole conversion, stderr: {}",
            self.stderr
        );
        assert!(self.pdf.starts_with(b"%PDF-"));
        assert!(
            !embedded_as_vector(&self.pdf),
            "a refused SVG must not end up in the PDF, stderr: {}",
            self.stderr
        );
    }
}

fn html_with(src: &str) -> String {
    format!(r#"<body style="margin:0"><img src="{src}"></body>"#)
}

// ===== 基準ディレクトリの中 =====

#[test]
fn a_plain_relative_reference_resolves_against_the_documents_directory() {
    let fx = Fixture::new("relative");
    fx.write("logo.svg", SVG);
    fx.write("in.html", &html_with("logo.svg"));
    fx.convert("in.html", &[]).assert_rendered();
}

#[test]
fn a_reference_into_a_subdirectory_resolves() {
    let fx = Fixture::new("subdir");
    fx.write("assets/logo.svg", SVG);
    fx.write("in.html", &html_with("assets/logo.svg"));
    fx.convert("in.html", &[]).assert_rendered();
}

/// `..`を含んでいても基準ディレクトリの中で収まるなら通す。
#[test]
fn a_parent_reference_that_stays_inside_the_base_directory_resolves() {
    let fx = Fixture::new("inside-parent");
    fx.write("logo.svg", SVG);
    fx.write("assets/in.html", &html_with("../logo.svg"));
    // base_dirは入力HTMLのあるディレクトリ(assets/)になるため、
    // `../logo.svg`はその外へ出る。`--allow`で基準ディレクトリを明示する。
    fx.convert("assets/in.html", &["--allow", fx.dir.to_str().unwrap()])
        .assert_rendered();
}

#[test]
fn a_dot_segment_inside_the_base_directory_resolves() {
    let fx = Fixture::new("dot-segment");
    fx.write("images/logo.svg", SVG);
    fx.write("in.html", &html_with("assets/../images/logo.svg"));
    fx.convert("in.html", &[]).assert_rendered();
}

/// ルート相対(`/logo.svg`)は「サイトルート」=基準ディレクトリとして解決する
/// (OSのファイルシステムルートを読みに行かない)。
#[test]
fn a_root_relative_reference_is_resolved_against_the_base_directory() {
    let fx = Fixture::new("root-relative");
    fx.write("logo.svg", SVG);
    fx.write("in.html", &html_with("/logo.svg"));
    fx.convert("in.html", &[]).assert_rendered();
}

#[test]
fn base_href_prefixes_a_relative_reference() {
    let fx = Fixture::new("base-href");
    fx.write("assets/logo.svg", SVG);
    fx.write(
        "in.html",
        r#"<head><base href="assets/"></head><body style="margin:0"><img src="logo.svg"></body>"#,
    );
    fx.convert("in.html", &[]).assert_rendered();
}

// ===== 基準ディレクトリの外・アクセス制御 =====

/// `<img src="../../secret.svg">`のような参照は既定で拒否する。
#[test]
fn a_reference_that_escapes_the_base_directory_is_refused() {
    let fx = Fixture::new("escape");
    fx.write("secret.svg", SVG);
    fx.write("public/in.html", &html_with("../secret.svg"));
    fx.convert("public/in.html", &[]).assert_refused();
    let reason = fx.refusal_reason("public/in.html", &[]);
    assert!(
        reason.contains("基準ディレクトリ"),
        "the reason should name the containment, got: {reason}"
    );
}

#[test]
fn stacked_parent_segments_do_not_slip_past_the_containment() {
    let fx = Fixture::new("stacked-escape");
    fx.write("secret.svg", SVG);
    fx.write("public/deep/in.html", &html_with("../../../secret.svg"));
    fx.convert("public/deep/in.html", &[]).assert_refused();
}

/// `--allow`で明示したディレクトリの中なら、基準ディレクトリの外でも読める。
#[test]
fn allow_permits_a_reference_outside_the_base_directory() {
    let fx = Fixture::new("allow");
    let outside = fx.write("outside/logo.svg", SVG);
    fx.write("public/in.html", &html_with("../outside/logo.svg"));
    fx.convert(
        "public/in.html",
        &["--allow", outside.parent().unwrap().to_str().unwrap()],
    )
    .assert_rendered();
}

/// `--allow`を付けても、指定したディレクトリの外は読めない。
#[test]
fn allow_does_not_permit_a_reference_outside_the_allowed_directory() {
    let fx = Fixture::new("allow-elsewhere");
    fx.write("secret/logo.svg", SVG);
    let permitted = fx.path("other");
    std::fs::create_dir_all(&permitted).unwrap();
    fx.write("public/in.html", &html_with("../secret/logo.svg"));
    let allow = ["--allow", permitted.to_str().unwrap()];
    fx.convert("public/in.html", &allow).assert_refused();
    let reason = fx.refusal_reason("public/in.html", &allow);
    assert!(
        reason.contains("--allow"),
        "the reason should mention --allow, got: {reason}"
    );
}

/// `--disable-local-file-access`は、基準ディレクトリの中の参照も拒否する
/// (サーバモードの既定)。
#[test]
fn disable_local_file_access_refuses_even_a_reference_inside_the_base_directory() {
    let fx = Fixture::new("no-local");
    fx.write("logo.svg", SVG);
    fx.write("in.html", &html_with("logo.svg"));
    let flag = ["--disable-local-file-access"];
    fx.convert("in.html", &flag).assert_refused();
    let reason = fx.refusal_reason("in.html", &flag);
    assert!(
        reason.contains("ローカルファイル"),
        "the reason should say local file access is off, got: {reason}"
    );
}

/// リモート取得は既定で無効。ネットワークへは出ないまま拒否される。
#[test]
fn a_remote_reference_is_refused_unless_remote_assets_are_allowed() {
    let fx = Fixture::new("remote");
    fx.write("in.html", &html_with("https://example.invalid/logo.svg"));
    fx.convert("in.html", &[]).assert_refused();
    let reason = fx.refusal_reason("in.html", &[]);
    assert!(
        reason.contains("--allow-remote-assets"),
        "the reason should point at the opt-in flag, got: {reason}"
    );
}

/// 取得できなかったときに文書ごと失敗させたい場合。
#[test]
fn load_media_error_handling_abort_fails_the_conversion_for_an_unreachable_svg() {
    let fx = Fixture::new("abort");
    fx.write("public/in.html", &html_with("../missing.svg"));
    let outcome = fx.convert("public/in.html", &["--load-media-error-handling", "abort"]);
    assert!(
        !outcome.success,
        "abort should make an unresolvable SVG fail the conversion, stderr: {}",
        outcome.stderr
    );
}

// ===== フォーマット判定はバイト列で行う =====

/// 拡張子は見ない。`.txt`の中身がSVGならSVGとして描く。
#[test]
fn the_extension_does_not_decide_the_format() {
    let fx = Fixture::new("ext-svg");
    fx.write("logo.txt", SVG);
    fx.write("in.html", &html_with("logo.txt"));
    fx.convert("in.html", &[]).assert_rendered();
}

/// 逆も同じ。`.svg`の中身がPNGならラスタ画像として描く。
#[test]
fn a_png_named_svg_is_still_embedded_as_a_raster_image() {
    let fx = Fixture::new("png-named-svg");
    let png = std::fs::read(PNG_PATH).expect("fixture image should exist");
    fx.write_bytes("logo.svg", &png);
    fx.write("in.html", &html_with("logo.svg"));
    let outcome = fx.convert("in.html", &[]);
    assert!(outcome.success, "stderr: {}", outcome.stderr);
    assert!(
        embedded_as_raster(&outcome.pdf),
        "PNG bytes should be embedded as an image XObject regardless of the .svg name"
    );
    assert!(
        !embedded_as_vector(&outcome.pdf),
        "PNG bytes must not be run through the SVG path"
    );
}

/// `data:`URIも同じ経路(バイト列を見る)。宣言mime typeが間違っていても通る。
#[test]
fn a_data_uri_svg_is_rendered_even_when_the_declared_mime_type_is_wrong() {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    let fx = Fixture::new("data-uri");
    let encoded = STANDARD.encode(SVG);
    // わざと`image/png`と名乗らせる。
    fx.write(
        "in.html",
        &html_with(&format!("data:image/png;base64,{encoded}")),
    );
    fx.convert("in.html", &[]).assert_rendered();
}

/// `%XX`をURLエンコードする(テスト用の最小実装。RFC 3986の
/// unreserved以外をすべてエスケープする)。
fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// SVGのdata URIは`;base64`ではなくパーセントエンコードで書くのが一般的。
/// base64しか受けないと、この最も普通の書き方が通らない。
#[test]
fn a_percent_encoded_svg_data_uri_is_rendered() {
    let fx = Fixture::new("data-uri-percent");
    fx.write(
        "in.html",
        &html_with(&format!("data:image/svg+xml,{}", percent_encode(SVG))),
    );
    fx.convert("in.html", &[]).assert_rendered();
}

/// `;utf8,`のような慣習的なパラメータが付いていても、`;base64`が無ければ
/// パーセントエンコードとして読む。
#[test]
fn a_data_uri_with_a_charset_style_parameter_but_no_base64_is_rendered() {
    let fx = Fixture::new("data-uri-utf8");
    fx.write(
        "in.html",
        &html_with(&format!("data:image/svg+xml;utf8,{}", percent_encode(SVG))),
    );
    fx.convert("in.html", &[]).assert_rendered();

    let charset = Fixture::new("data-uri-charset");
    charset.write(
        "in.html",
        &html_with(&format!(
            "data:image/svg+xml;charset=utf-8,{}",
            percent_encode(SVG)
        )),
    );
    charset.convert("in.html", &[]).assert_rendered();
}

/// CSSの`url()`の中でも同じ(`background-image`は`<img src>`と同じ経路を通る)。
#[test]
fn a_percent_encoded_svg_data_uri_works_in_a_css_url() {
    let fx = Fixture::new("data-uri-css");
    fx.write(
        "in.html",
        &format!(
            r#"<body style="margin:0"><div style="width:60px;height:30px;
                 background-image:url('data:image/svg+xml,{}');
                 background-repeat:no-repeat"></div></body>"#,
            percent_encode(SVG)
        ),
    );
    fx.convert("in.html", &[]).assert_rendered();
}

/// エンコードされていない生のSVGも通す(空白を落とさないことの確認も兼ねる。
/// 落とすと`<svgxmlns=...>`になってパースできない)。HTML属性の中では引用符が
/// 衝突するので、属性は単引用符で囲む。
#[test]
fn an_unencoded_svg_data_uri_is_rendered() {
    let fx = Fixture::new("data-uri-raw");
    fx.write(
        "in.html",
        &format!(r#"<body style="margin:0"><img src='data:image/svg+xml,{SVG}'></body>"#),
    );
    fx.convert("in.html", &[]).assert_rendered();
}

/// gzip圧縮されたSVG(`.svgz`)。マジックバイト`1f 8b`で嗅ぎ分け、
/// 展開はusvgに任せる。
#[test]
fn a_gzipped_svgz_is_rendered() {
    use std::io::Write;

    let fx = Fixture::new("svgz");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(SVG.as_bytes()).unwrap();
    let gzipped = encoder.finish().unwrap();
    assert_eq!(&gzipped[..2], &[0x1F, 0x8B], "fixture should be gzip");

    fx.write_bytes("logo.svgz", &gzipped);
    fx.write("in.html", &html_with("logo.svgz"));
    fx.convert("in.html", &[]).assert_rendered();
}

/// `background-image: url(...)`も`<img src>`と同じ解決・封じ込めを通る。
#[test]
fn a_background_image_reference_uses_the_same_containment() {
    let fx = Fixture::new("background");
    fx.write("secret.svg", SVG);
    fx.write(
        "public/in.html",
        r#"<body style="margin:0"><div style="width:60px;height:30px;
             background-image:url(../secret.svg);background-repeat:no-repeat"></div></body>"#,
    );
    fx.convert("public/in.html", &[]).assert_refused();

    let ok = Fixture::new("background-ok");
    ok.write("logo.svg", SVG);
    ok.write(
        "in.html",
        r#"<body style="margin:0"><div style="width:60px;height:30px;
             background-image:url(logo.svg);background-repeat:no-repeat"></div></body>"#,
    );
    ok.convert("in.html", &[]).assert_rendered();
}

/// 同じSVGを別の書き方で参照しても、解決後が同じファイルなら取得は1回。
/// (`src`文字列がキーなので、書き方が違えば別エントリになる。ここで見たいのは
/// 「同じ文字列なら1回」の方。)
#[test]
fn the_same_reference_is_fetched_once() {
    let fx = Fixture::new("dedup");
    fx.write("logo.svg", SVG);
    fx.write(
        "in.html",
        r#"<body style="margin:0">
             <img src="logo.svg"><img src="logo.svg"><img src="logo.svg">
           </body>"#,
    );
    let outcome = fx.convert("in.html", &[]);
    outcome.assert_rendered();
    assert_eq!(
        count_occurrences(&outcome.pdf, b"/Subtype /Form"),
        1,
        "three references to the same file should share one form XObject"
    );
}

/// 参照先がディレクトリだった場合に、変換ごと落ちたりしないこと。
#[test]
fn a_reference_to_a_directory_is_refused_without_crashing() {
    let fx = Fixture::new("dir-ref");
    std::fs::create_dir_all(fx.path("logo.svg")).unwrap();
    fx.write("in.html", &html_with("logo.svg"));
    fx.convert("in.html", &[]).assert_refused();
}

/// 空ファイルもSVGとしては解釈できない。警告して先へ進む。
#[test]
fn an_empty_file_is_refused_without_crashing() {
    let fx = Fixture::new("empty");
    fx.write("logo.svg", "");
    fx.write("in.html", &html_with("logo.svg"));
    fx.convert("in.html", &[]).assert_refused();
}
