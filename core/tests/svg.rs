//! `<img src="*.svg">`と`background-image: url(*.svg)`のE2Eテスト。
//!
//! SVGはラスタ画像と違い「Form XObject 1つ + その参照先」という複数
//! オブジェクトのかたまりとしてPDFへ入る。そのため確認したいのは主に2点:
//!
//! 1. ラスタライズせずベクタのまま入っていること(Image XObjectではなく
//!    Form XObjectになり、パス描画演算子が現れる)
//! 2. 複数オブジェクトを差し込んでもxrefが壊れないこと
//!
//! 2つ目は書き出し方が2通りあるので両方を通す。ライブラリの`encode_pdf`は
//! `Chunk::extend`でオフセットを付け替えるが、`Sink`へ書く経路
//! (`StreamingPdfWriter`、CLIは`--streaming`の有無に関わらずこちらを使う)は
//! 自前でxrefを組むため、チャンク内の各オブジェクトの位置を数える必要がある。
//!
//! 描画結果のピクセル比較はしない(svg2pdf側の責務)。ここで見るのは
//! 「この処理系がSVGをPDFの正しい構造として繋げられているか」だけ。

#![cfg(feature = "svg")]

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{
    paginate_document_with_absolutes, resolve_background_images, PageSettings,
};
use sghtmltopdf_core::pdf::{encode_pdf, ImageAssetCache};
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
const BIN: &str = env!("CARGO_BIN_EXE_sghtmltopdf");

/// 20x10のSVG。塗りとストロークだけで、フォントにもラスタ画像にも依存しない。
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
  <rect x="0" y="0" width="20" height="10" fill="#0000ff"/>
  <circle cx="10" cy="5" r="4" fill="#ff0000" stroke="#00ff00" stroke-width="1"/>
</svg>"##;

/// グラデーションと`opacity`を持つSVG。svg2pdfのチャンクがShading・Pattern・
/// ExtGState・ICCBasedのストリーム・入れ子のForm XObjectまで含むようになり、
/// **オブジェクト番号の順序とバイト列上の順序が一致しなくなる**
/// (グラデーションの本体の中から参照されるICCプロファイルは、番号は先に
/// 振られるがチャンクの末尾に書かれる)。xrefのオフセットを数える処理で
/// いちばん壊れやすいのがこのケースなので、意図して用意している。
const SVG_WITH_GRADIENT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60" width="100" height="60">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ff5500"/>
      <stop offset="1" stop-color="#0055ff"/>
    </linearGradient>
  </defs>
  <rect x="2" y="2" width="96" height="56" rx="8" fill="url(#g)" stroke="#222" stroke-width="2"/>
  <circle cx="30" cy="30" r="16" fill="#fff" opacity="0.7"/>
  <path d="M60 12 L88 48 L60 48 Z" fill="#0a0" stroke="#050" stroke-width="2"/>
</svg>"##;

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|at| from + at)
}

/// content streamはFlateDecodeで圧縮されているため、演算子を探すには解凍が
/// 必要(`tests/background.rs`の同名関数と同じロジック)。
fn decompressed_stream_bytes(pdf_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find_from(pdf_bytes, b"stream\n", i) {
        let start = pos + b"stream\n".len();
        let Some(end) = find_from(pdf_bytes, b"\nendstream", start) else {
            break;
        };
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

/// xrefテーブルの各エントリが本当にそのオブジェクトの先頭を指しているか
/// 確認する。SVGを差し込むとオブジェクトが一度に何個も増えるため、
/// オフセットが1つでもずれるとPDF全体が読めなくなる。
///
/// 併せて`/Size`とエントリ数が合っているか(=1から連番で全て書かれて
/// いるか)も見る。ストリーミング書き出しのxrefはその前提で組まれている。
fn assert_xref_is_consistent(pdf: &[u8]) {
    let startxref = rfind(pdf, b"startxref").expect("PDF should end with startxref");
    let xref_offset: usize = ascii_number_after(pdf, startxref + b"startxref".len())
        .expect("startxref should be followed by an offset");
    assert!(
        pdf[xref_offset..].starts_with(b"xref"),
        "startxref should point at the xref table"
    );

    // `xref\n0 {size}\n`のあと、1行20バイトのエントリが並ぶ。
    let header_end = find_from(pdf, b"\n", xref_offset + b"xref".len() + 1)
        .expect("xref subsection header should end with a newline");
    let subsection = std::str::from_utf8(&pdf[xref_offset + b"xref\n".len()..header_end])
        .expect("xref subsection header should be ASCII");
    let (first, size) = subsection
        .split_once(' ')
        .expect("xref subsection header should be `first count`");
    assert_eq!(first, "0", "the subsection should start at object 0");
    let size: usize = size.trim().parse().expect("count should be a number");

    // エントリは1件20バイト固定(`nnnnnnnnnn ggggg t` + 2バイトの行末)。
    // 先頭はオブジェクト0のフリーエントリ。実オブジェクトは1..size。
    let entries_start = header_end + 1;
    let mut in_use = 0;
    for id in 1..size {
        let entry_at = entries_start + id * 20;
        let entry = std::str::from_utf8(&pdf[entry_at..entry_at + 20])
            .unwrap_or_else(|_| panic!("xref entry for object {id} should be ASCII"));
        // 使われていない番号は`f`(free)エントリになる。`encode_pdf`は払い出した
        // まま使わない番号を残すことがあるので、`n`だけを検証する。
        if entry.as_bytes()[17] == b'f' {
            continue;
        }
        assert_eq!(
            entry.as_bytes()[17],
            b'n',
            "xref entry for object {id} should be marked `n` or `f`, got {entry:?}"
        );
        in_use += 1;
        let offset: usize = entry[..10]
            .parse()
            .unwrap_or_else(|_| panic!("xref entry for object {id} should start with an offset"));
        let expected = format!("{id} 0 obj");
        assert!(
            pdf[offset..].starts_with(expected.as_bytes()),
            "xref says object {id} is at {offset}, but that is not where `{expected}` starts \
             (found {:?})",
            String::from_utf8_lossy(&pdf[offset..(offset + 24).min(pdf.len())])
        );
    }

    // ファイル中の`N 0 obj`の総数と`n`エントリ数が一致すること
    // (=xrefに載っていないオブジェクトが無い)。
    let written = (1..size)
        .filter(|id| count_occurrences(pdf, format!("\n{id} 0 obj\n").as_bytes()) > 0)
        .count();
    assert_eq!(
        written, in_use,
        "every object written to the file should have an in-use xref entry"
    );
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn ascii_number_after(bytes: &[u8], from: usize) -> Option<usize> {
    let rest = &bytes[from..];
    let start = rest.iter().position(|b| b.is_ascii_digit())?;
    let end = rest[start..]
        .iter()
        .position(|b| !b.is_ascii_digit())
        .map_or(rest.len(), |i| start + i);
    std::str::from_utf8(&rest[start..end]).ok()?.parse().ok()
}

/// HTMLとSVGを一時ディレクトリへ書き、CLIを通してPDFへ変換する。
/// `extra`に`--streaming`を渡せばストリーミングのページ確定になる。
fn convert(html: &str, extra: &[&str], name: &str) -> Vec<u8> {
    convert_svg_file(html, SVG, extra, name)
}

fn convert_svg_file(html: &str, svg: &str, extra: &[&str], name: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("sghtmltopdf-svg-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("logo.svg"), svg).unwrap();
    let input = dir.join("input.html");
    std::fs::write(&input, html).unwrap();
    let output = dir.join("out.pdf");

    let result = Command::new(BIN)
        .arg(&input)
        .args(["--font", FONT_PATH])
        .args(extra)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("failed to run the sghtmltopdf binary");
    assert!(
        result.status.success(),
        "CLI should succeed, stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !stderr.contains("警告"),
        "converting an SVG should not warn, got: {stderr}"
    );

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
    cleanup(&dir);
    bytes
}

fn cleanup(dir: &Path) {
    std::fs::remove_dir_all(dir).ok();
}

/// SVGがベクタのまま入っている印。ラスタ化されていれば
/// `/Subtype /Image`になり、`/Subtype /Form`もパス演算子も出ない。
fn assert_embedded_as_vector(pdf: &[u8]) {
    assert!(
        count_occurrences(pdf, b"/Subtype /Form") > 0,
        "an SVG should become a form XObject"
    );
    assert_eq!(
        count_occurrences(pdf, b"/Subtype /Image"),
        0,
        "an SVG must not be rasterised into an image XObject"
    );

    let content = decompressed_stream_bytes(pdf);
    assert!(
        count_occurrences(&content, b" c\n") > 0 || count_occurrences(&content, b" c ") > 0,
        "the circle should be drawn as Bezier curves in a content stream"
    );
    assert!(
        count_occurrences(&content, b" Do\n") > 0,
        "the form XObject should be invoked with Do"
    );
}

#[test]
fn an_img_pointing_at_an_svg_is_embedded_as_vector_graphics() {
    let pdf = convert(
        r#"<body style="margin:0"><img src="logo.svg"></body>"#,
        &[],
        "img",
    );
    assert_embedded_as_vector(&pdf);
    assert_xref_is_consistent(&pdf);
}

/// `<img>`に寸法が無ければSVGの内在サイズ(20x10)でレイアウトされる。
/// `--zoom 1`・既定スケール0.75のもとで、幅20pxのボックスは`20 0 0 10`の
/// `cm`として現れる。
#[test]
fn an_svg_without_attributes_lays_out_at_its_intrinsic_size() {
    let pdf = convert(
        r#"<body style="margin:0"><img src="logo.svg"></body>"#,
        &[],
        "intrinsic",
    );
    let content = decompressed_stream_bytes(&pdf);
    let text = String::from_utf8_lossy(&content);
    assert!(
        text.contains("20 0 0 10 "),
        "the unit-square form XObject should be scaled to the SVG's intrinsic 20x10, \
         content was: {text}"
    );
}

/// 属性で寸法を与えたら内在サイズではなくそちらが効く(単位正方形へ
/// 正規化されているので、ラスタと同じ`cm`だけで拡縮できる)。
#[test]
fn width_and_height_attributes_scale_the_svg() {
    let pdf = convert(
        r#"<body style="margin:0"><img src="logo.svg" width="100" height="50"></body>"#,
        &[],
        "scaled",
    );
    let content = decompressed_stream_bytes(&pdf);
    let text = String::from_utf8_lossy(&content);
    assert!(
        text.contains("100 0 0 50 "),
        "the form XObject should be scaled to 100x50, content was: {text}"
    );
}

#[test]
fn an_svg_works_as_a_background_image() {
    let pdf = convert(
        r#"<body style="margin:0">
             <div style="width:60px;height:30px;background-image:url(logo.svg);
                         background-repeat:no-repeat"></div>
           </body>"#,
        &[],
        "background",
    );
    assert_embedded_as_vector(&pdf);
    assert_xref_is_consistent(&pdf);
}

/// ストリーミング書き出しはxrefを自前で組むため、SVGの複数オブジェクトが
/// 入ってもオフセットがずれないことを確認する(この検証がこのファイルの
/// 主目的)。
#[test]
fn streaming_mode_writes_a_consistent_xref_with_an_svg() {
    let pdf = convert(
        r#"<body style="margin:0"><img src="logo.svg"></body>"#,
        &["--streaming"],
        "streaming",
    );
    assert_embedded_as_vector(&pdf);
    assert_xref_is_consistent(&pdf);
}

/// 同じSVGを何度参照してもForm XObjectは1つしか書かれない
/// (ラスタ画像と同じ`Rc`単位のメモ化が効く)。
#[test]
fn the_same_svg_referenced_twice_is_embedded_once() {
    let pdf = convert(
        r#"<body style="margin:0">
             <img src="logo.svg"><img src="logo.svg"><img src="logo.svg">
           </body>"#,
        &[],
        "dedup",
    );
    assert_eq!(
        count_occurrences(&pdf, b"/Subtype /Form"),
        1,
        "three references to the same SVG should share one form XObject"
    );
    assert_xref_is_consistent(&pdf);
}

/// グラデーション入りのSVGはオブジェクト番号順とバイト列上の順序が
/// 食い違う([`SVG_WITH_GRADIENT`]のコメント参照)。`Sink`経路のxrefが
/// それでも正しく組めることを確認する。
#[test]
fn a_gradient_svg_keeps_the_xref_consistent_when_object_order_is_not_monotonic() {
    for (mode, name) in [
        (&[][..], "gradient-batch"),
        (&["--streaming"][..], "gradient-streaming"),
    ] {
        let pdf = convert_svg_file(
            r#"<body style="margin:0"><img src="logo.svg" width="300" height="180"></body>"#,
            SVG_WITH_GRADIENT,
            mode,
            name,
        );
        assert_embedded_as_vector(&pdf);
        assert_xref_is_consistent(&pdf);
        // グラデーションがベクタのまま(Shadingとして)入っている印。
        assert!(
            count_occurrences(&pdf, b"/ShadingType") > 0,
            "the linear gradient should stay a PDF shading, not be flattened, in {name}"
        );
    }
}

/// ライブラリの`encode_pdf`はCLIとは別の書き出し経路(`Chunk::extend`)を
/// 通るため、こちらも独立に確認する。
#[test]
fn encode_pdf_embeds_an_svg_with_a_consistent_xref() {
    let svg_data_uri = format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(SVG_WITH_GRADIENT)
    );
    let html_src = r#"<body style="margin:0"><img src="PLACEHOLDER"></body>"#
        .replace("PLACEHOLDER", &svg_data_uri);

    let mut dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet("");
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load the test font")
    ]);
    let settings = PageSettings::default();

    // data URIしか使わないのでbase_dirは何でもよい(I/Oは起きない)。
    let image_cache = ImageAssetCache::new(PathBuf::from("."), false);
    let background_images = resolve_background_images(&styles, &image_cache);
    let pages =
        paginate_document_with_absolutes(&mut dom, &styles, &fonts, &settings, &image_cache);
    let pdf = encode_pdf(&pages, &styles, &background_images, &fonts, &settings);

    assert!(pdf.starts_with(b"%PDF-"));
    assert_embedded_as_vector(&pdf);
    assert_xref_is_consistent(&pdf);
}

/// SVGの中の`<image href="...">`からファイルを読ませない。
///
/// usvgの既定の解決関数はhrefをそのまま`std::fs::read`するため、`<img>`側の
/// 封じ込め(基準ディレクトリ・`--allow`・`--disable-local-file-access`)を
/// 迂回できてしまう。`pdf::svg`でこれを差し替えているので、参照が拒否され、
/// 読まれた中身が一切PDFへ出ないことを確認する。
#[test]
fn an_svg_cannot_read_files_through_a_nested_image_href() {
    let dir = std::env::temp_dir().join(format!("sghtmltopdf-svg-{}-exfil", std::process::id()));
    let public = dir.join("public");
    std::fs::create_dir_all(&public).unwrap();

    // base_dirの外に置いた「秘密の」SVG。マゼンタの塗り(`1 0 1 rg`)が
    // PDFへ出てきたら中身が漏れたということ。
    std::fs::write(
        dir.join("secret.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="20">
              <rect width="100" height="20" fill="#ff00ff"/>
            </svg>"##,
    )
    .unwrap();

    let evil = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">
             <rect width="200" height="60" fill="#dddddd"/>
             <image href="../secret.svg" x="0" y="0" width="100" height="20"/>
             <image href="{}" x="0" y="30" width="100" height="20"/>
             <image href="/etc/passwd" x="100" y="0" width="50" height="20"/>
           </svg>"##,
        dir.join("secret.svg").display()
    );
    std::fs::write(public.join("evil.svg"), evil).unwrap();
    let input = public.join("input.html");
    std::fs::write(&input, r#"<body><img src="evil.svg"></body>"#).unwrap();
    let output = public.join("out.pdf");

    let result = Command::new(BIN)
        .arg(&input)
        .args(["--font", FONT_PATH])
        .arg("-o")
        .arg(&output)
        .output()
        .expect("failed to run the sghtmltopdf binary");
    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("SVG内の外部参照は読み込みません"),
        "each nested href should be refused with a warning, got: {stderr}"
    );

    let pdf = std::fs::read(&output).unwrap();
    let content = decompressed_stream_bytes(&pdf);
    assert_eq!(
        count_occurrences(&content, b"1 0 1 rg"),
        0,
        "the referenced SVG's magenta fill must not appear in the PDF"
    );
    assert_eq!(
        count_occurrences(&pdf, b"root:"),
        0,
        "/etc/passwd must not appear in the PDF"
    );
    // 参照を拒否しても、SVG自身(グレーの背景)は描かれる。
    assert!(count_occurrences(&pdf, b"/Subtype /Form") > 0);

    cleanup(&dir);
}

/// 壊れたSVGは画像なしの置換要素として扱われ、変換自体は成功する
/// (ラスタのデコード失敗と同じ扱い)。
#[test]
fn a_broken_svg_does_not_abort_the_conversion() {
    let dir = std::env::temp_dir().join(format!("sghtmltopdf-svg-{}-broken", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("logo.svg"), "<svg><this is not xml").unwrap();
    let input = dir.join("input.html");
    std::fs::write(&input, r#"<body><img src="logo.svg"><p>after</p></body>"#).unwrap();
    let output = dir.join("out.pdf");

    let result = Command::new(BIN)
        .arg(&input)
        .args(["--font", FONT_PATH])
        .arg("-o")
        .arg(&output)
        .output()
        .expect("failed to run the sghtmltopdf binary");
    assert!(
        result.status.success(),
        "a broken SVG should not fail the conversion by default, stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let bytes = std::fs::read(&output).expect("output PDF should exist");
    assert!(bytes.starts_with(b"%PDF-"));
    assert_xref_is_consistent(&bytes);
    cleanup(&dir);
}
