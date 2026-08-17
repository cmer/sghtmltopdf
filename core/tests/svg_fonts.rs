//! SVG内の`<text>`が使うフォントのE2Eテスト。
//!
//! 要求は「HTML文書で使えるフォントは、翻訳後のPDFチャンクの中でも使える」
//! こと。SVGの翻訳(svg2pdf)はこの処理系とは別のフォント解決の仕組み
//! (usvgの`fontdb`)を持っているため、そこへ文書のフォントを渡してやらないと
//! **HTMLでは出るのにSVGの中だけ字が消える**という食い違いが起きる。
//! [`SvgFontDb`]がその橋渡しで、ここではその両端を確認する。
//!
//! 確認の仕方: svg2pdfが埋め込むフォントは`/BaseFont /TAG+FamilyName`
//! (6文字のサブセットタグ + `+` + family名)という名前になる。この処理系
//! 自身のフォント書き出しは`/BaseFont /EmbeddedFont`で、タグを持たない。
//! したがって「`+`を含む`/BaseFont`」の有無で、SVG側にフォントが渡って
//! 埋め込まれたかどうかが分かる。
//!
//! `svg-text` featureが無い場合はSVG内のテキストを描かない(そのぶん
//! rustybuzz・resvg等を引き込まない)。その場合の挙動もここで確認する。

#![cfg(feature = "svg")]

use std::path::PathBuf;
use std::process::Command;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::pdf::SvgFontDb;

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
const BOLD_FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fonts/DejaVuSans-Bold.ttf"
);
const BIN: &str = env!("CARGO_BIN_EXE_sghtmltopdf");

/// `DejaVuSans.ttf`が`name`テーブルで名乗っているfamily名。
const INTERNAL_FAMILY: &str = "DejaVu Sans";

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// PDF中の`/BaseFont /...`の値をすべて集める。
fn base_font_names(pdf: &[u8]) -> Vec<String> {
    const KEY: &[u8] = b"/BaseFont /";
    let mut names = Vec::new();
    let mut i = 0;
    while i + KEY.len() <= pdf.len() {
        if !pdf[i..].starts_with(KEY) {
            i += 1;
            continue;
        }
        let start = i + KEY.len();
        let end = pdf[start..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || *b == b'+' || *b == b'-'))
            .map_or(pdf.len(), |n| start + n);
        names.push(String::from_utf8_lossy(&pdf[start..end]).into_owned());
        i = end;
    }
    names
}

/// svg2pdfが埋め込んだフォントの名前(サブセットタグ`TAG+`が付いているもの)。
/// この処理系自身の書き出しは`EmbeddedFont`でタグを持たないので混ざらない。
///
/// `/BaseFont`はType0辞書とCIDFont辞書の両方に書かれるので、1つのフォントでも
/// 2回現れる。重複は畳んで「何種類埋め込まれたか」を返す。
fn svg_embedded_fonts(pdf: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = base_font_names(pdf)
        .into_iter()
        .filter(|name| name.contains('+'))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// 埋め込まれたフォントプログラム(`/FontFile2`)の数。サブセット1つにつき1本。
fn embedded_font_programs(pdf: &[u8]) -> usize {
    count_occurrences(pdf, b"/FontFile2")
}

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("sghtmltopdf-svgfont-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
    }

    #[cfg_attr(not(feature = "svg-text"), allow(dead_code))]
    fn copy_font(&self, from: &str, to: &str) {
        std::fs::copy(from, self.dir.join(to)).unwrap();
    }

    fn convert(&self, extra: &[&str]) -> Vec<u8> {
        let output = self.dir.join("out.pdf");
        let result = Command::new(BIN)
            .arg(self.dir.join("in.html"))
            .args(extra)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("failed to run the sghtmltopdf binary");
        assert!(
            result.status.success(),
            "the conversion should succeed, stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let pdf = std::fs::read(&output).expect("output PDF should exist");
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(
            count_occurrences(&pdf, b"/Subtype /Form") > 0,
            "the SVG itself should always be embedded, whatever happens to its text"
        );
        pdf
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// `<text>`を1つ持つSVG。`family`が`None`なら`font-family`を書かない。
fn svg_with_text(family: Option<&str>) -> String {
    let family_attr = family
        .map(|f| format!(r#" font-family="{f}""#))
        .unwrap_or_default();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40">
              <rect width="200" height="40" fill="#eeeeee"/>
              <text x="5" y="25"{family_attr} font-size="16" fill="#000000">Hamburgefonstiv</text>
            </svg>"##
    )
}

// ===== `SvgFontDb`(橋渡しそのもの) =====

/// 文書のフォントコレクションから組んだデータベースには、コレクションにある
/// フォントが入る。`svg-text`が無効なときは空(SVG内のテキストを描かない)。
#[test]
fn the_font_db_is_built_from_the_documents_font_collection() {
    let collection = FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load the test font"),
        Font::load(BOLD_FONT_PATH).expect("should load the bold test font"),
    ]);
    let db = SvgFontDb::from_collection(&collection);

    if cfg!(feature = "svg-text") {
        assert_eq!(
            db.len(),
            collection.len(),
            "every font available to the document should be available to the SVG"
        );
    } else {
        assert!(
            db.is_empty(),
            "without `svg-text` the SVG font db stays empty"
        );
    }
}

#[test]
fn an_empty_font_db_has_no_faces() {
    assert!(SvgFontDb::empty().is_empty());
    assert_eq!(SvgFontDb::empty().len(), 0);
}

/// フォントを持たない文書から組んでも空になるだけで、パニックしない。
#[test]
fn an_empty_collection_produces_an_empty_font_db() {
    let db = SvgFontDb::from_collection(&FontCollection::new(Vec::new()));
    assert!(db.is_empty());
}

// ===== `svg-text`が有効なとき: 文書のフォントがSVGへ渡る =====

/// `--font`で渡したフォントを、SVGの中からフォント内部のfamily名で引ける。
#[cfg(feature = "svg-text")]
#[test]
fn a_font_given_to_the_document_is_usable_from_the_svg_by_its_internal_family_name() {
    let fx = Fixture::new("internal-name");
    fx.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    fx.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let pdf = fx.convert(&["--font", FONT_PATH]);

    let embedded = svg_embedded_fonts(&pdf);
    assert!(
        embedded.iter().any(|name| name.contains("DejaVuSans")),
        "the document's font should be embedded for the SVG's text, got {embedded:?}"
    );
}

/// `font-family`を書かない`<text>`は、usvgの既定("Times New Roman")ではなく
/// 文書の既定フォントで描く。手元に無い名前を既定に据えると、指定の無い
/// テキストが必ず消えてしまう。
#[cfg(feature = "svg-text")]
#[test]
fn text_without_a_font_family_falls_back_to_the_documents_font() {
    let fx = Fixture::new("default-family");
    fx.write("logo.svg", &svg_with_text(None));
    fx.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let pdf = fx.convert(&["--font", FONT_PATH]);

    let embedded = svg_embedded_fonts(&pdf);
    assert!(
        embedded.iter().any(|name| name.contains("DejaVuSans")),
        "text with no font-family should use the document's font, got {embedded:?}"
    );
}

/// `@font-face`で宣言した名前でもSVGから引ける。フォント内部の`name`テーブルは
/// `DejaVu Sans`なので、宣言名`BrandFace`は別名として登録されていないと
/// 解決できない。
#[cfg(feature = "svg-text")]
#[test]
fn a_font_face_declared_family_name_is_usable_from_the_svg() {
    let fx = Fixture::new("declared-name");
    fx.copy_font(FONT_PATH, "brand.ttf");
    fx.write("logo.svg", &svg_with_text(Some("BrandFace")));
    fx.write(
        "in.html",
        r#"<style>
             @font-face { font-family: BrandFace; src: url(brand.ttf); }
             body { margin: 0; font-family: BrandFace; }
           </style>
           <body><img src="logo.svg" width="200" height="40"></body>"#,
    );
    // `--font`は渡さない。SVGから引けるフォントは`@font-face`のものだけになる。
    let pdf = fx.convert(&[]);

    let embedded = svg_embedded_fonts(&pdf);
    assert!(
        !embedded.is_empty(),
        "the @font-face font should be embedded for the SVG's text, got {embedded:?}"
    );
}

/// 文書が持っていないfamilyは、勝手に別のフォントで代用しない
/// (システムフォントを探しに行かないため)。SVG自体は描かれる。
#[cfg(feature = "svg-text")]
#[test]
fn a_family_the_document_does_not_have_is_not_substituted() {
    let fx = Fixture::new("unknown-family");
    fx.write(
        "logo.svg",
        &svg_with_text(Some("NoSuchFamilyExistsAnywhere")),
    );
    fx.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let pdf = fx.convert(&["--font", FONT_PATH]);

    assert!(
        svg_embedded_fonts(&pdf).is_empty(),
        "an unknown family should not be silently replaced by another font"
    );
}

/// 文書側のフォント埋め込みとSVG側のフォント埋め込みはそれぞれ独立に行われる。
/// 同じフォントファイルでも、サブセットは別(必要なグリフが違う)。
#[cfg(feature = "svg-text")]
#[test]
fn the_svg_text_font_is_embedded_alongside_the_documents_own_font() {
    let fx = Fixture::new("both-embedded");
    fx.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    fx.write(
        "in.html",
        r#"<body style="margin:0"><p>document text</p>
             <img src="logo.svg" width="200" height="40"></body>"#,
    );
    let pdf = fx.convert(&["--font", FONT_PATH]);

    // 文書側(`/BaseFont /EmbeddedFont`)とSVG側(タグ付き)の両方がある。
    let names = base_font_names(&pdf);
    assert!(
        names.iter().any(|n| n == "EmbeddedFont"),
        "the document's own text should still embed its font, got {names:?}"
    );
    assert!(
        !svg_embedded_fonts(&pdf).is_empty(),
        "the SVG's text should embed its own subset, got {names:?}"
    );
    assert!(
        embedded_font_programs(&pdf) >= 2,
        "two independent subsets should be embedded"
    );
}

/// 同じSVGを複数回参照しても、フォントの埋め込みは1回だけ
/// (SVGのチャンクごと共有される)。
#[cfg(feature = "svg-text")]
#[test]
fn a_repeated_svg_does_not_embed_its_font_twice() {
    let one = Fixture::new("repeat-one");
    one.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    one.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let single = one.convert(&["--font", FONT_PATH]);

    let three = Fixture::new("repeat-three");
    three.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    three.write(
        "in.html",
        r#"<body style="margin:0">
             <img src="logo.svg" width="200" height="40">
             <img src="logo.svg" width="200" height="40">
             <img src="logo.svg" width="200" height="40">
           </body>"#,
    );
    let repeated = three.convert(&["--font", FONT_PATH]);

    assert_eq!(
        svg_embedded_fonts(&repeated).len(),
        1,
        "the shared SVG chunk should carry exactly one font subset"
    );
    assert_eq!(
        embedded_font_programs(&repeated),
        embedded_font_programs(&single),
        "referencing the same SVG three times should not embed its font program again"
    );
}

/// ストリーミング書き出しでも同じこと(フォントを含むSVGのチャンクは
/// オブジェクトが更に増えるので、xrefが崩れやすい経路でもある)。
#[cfg(feature = "svg-text")]
#[test]
fn streaming_mode_also_embeds_the_documents_font_for_svg_text() {
    let fx = Fixture::new("streaming");
    fx.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    fx.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let pdf = fx.convert(&["--font", FONT_PATH, "--streaming"]);

    assert!(
        !svg_embedded_fonts(&pdf).is_empty(),
        "streaming mode should embed the SVG's font too"
    );
}

// ===== `svg-text`が無効なとき =====

/// テキストは描かれないが、SVGの他の図形は描かれ、変換は成功する。
#[cfg(not(feature = "svg-text"))]
#[test]
fn without_the_svg_text_feature_the_text_is_dropped_but_the_svg_still_renders() {
    let fx = Fixture::new("no-text-feature");
    fx.write("logo.svg", &svg_with_text(Some(INTERNAL_FAMILY)));
    fx.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    // `convert`が`/Subtype /Form`の存在を確かめている(=矩形は描かれている)。
    let pdf = fx.convert(&["--font", FONT_PATH]);

    assert!(
        svg_embedded_fonts(&pdf).is_empty(),
        "without `svg-text` no font should be embedded for the SVG"
    );

    // テキストの無い同じ大きさのSVGと比べて、埋め込まれるフォントプログラムの
    // 数が変わらないこと(=`<text>`が何も足していない)。
    let plain = Fixture::new("no-text-feature-plain");
    plain.write(
        "logo.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40">
              <rect width="200" height="40" fill="#eeeeee"/>
            </svg>"##,
    );
    plain.write(
        "in.html",
        r#"<body style="margin:0"><img src="logo.svg" width="200" height="40"></body>"#,
    );
    let plain_pdf = plain.convert(&["--font", FONT_PATH]);
    assert_eq!(
        embedded_font_programs(&pdf),
        embedded_font_programs(&plain_pdf),
        "a dropped <text> should not embed a font program"
    );
}
