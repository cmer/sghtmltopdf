//! T9スパイク: pdf-writerでTrueTypeフォントを埋め込み、実際のグリフでテキストを
//! 描画したPDFを生成するPoC。
//!
//! 検証したいこと:
//! - CIDFontType2(Identity-H encoding)としてTrueTypeフォントを埋め込み、
//!   T7の`shape_text()`が返すグリフID列をそのままPDFのテキスト描画に使えるか
//! - base14フォント(T1のspike_krillaやspike_pdf_writer)ではなく、実際に
//!   埋め込んだフォントのグリフが表示されるか(アクセント付き文字を含めて確認)
//!
//! 実行: `cargo run --example spike_pdf_font_embedding`
//! (T7で追加したbundled test font `core/tests/fonts/DejaVuSans.ttf`を使用)

use std::collections::BTreeMap;

use pdf_writer::{Content, Finish, Name, Pdf, Rect as PdfRect, Ref, Str};
use sghtmltopdf_core::fonts::{shape_text, Font};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn main() {
    let font = Font::load(FONT_PATH).expect("should load bundled test font");

    let text = "Hello, world! caf\u{e9} r\u{e9}sum\u{e9}";
    let font_size = 24.0;
    let shaped = shape_text(&font, text, font_size);

    // Identity-H: 2バイトのコードをそのままCID(=GlyphID, CIDToGIDMap=Identity)として扱う。
    let mut glyph_bytes = Vec::with_capacity(shaped.glyphs.len() * 2);
    for g in &shaped.glyphs {
        glyph_bytes.extend_from_slice(&g.glyph_id.to_be_bytes());
    }

    // PDFの/Wは1000unit/emのグリフ空間で表現する。
    let units_per_em = font.units_per_em() as f32;
    let to_1000 = |font_units: f32| font_units * 1000.0 / units_per_em;

    // 使用したグリフごとの幅(グリフIDは連続していないので、個別に記録する)。
    let mut widths: BTreeMap<u16, f32> = BTreeMap::new();
    for g in &shaped.glyphs {
        widths.entry(g.glyph_id).or_insert_with(|| {
            let advance = font.glyph_hor_advance(g.glyph_id).unwrap_or(0) as f32;
            to_1000(advance)
        });
    }

    let mut ids = 0..;
    let mut next_id = || Ref::new(ids.next().unwrap() + 1);

    let catalog_id = next_id();
    let pages_tree_id = next_id();
    let page_id = next_id();
    let content_id = next_id();
    let font_file_id = next_id();
    let descriptor_id = next_id();
    let cid_font_id = next_id();
    let type0_font_id = next_id();

    let mut pdf = Pdf::new();

    pdf.catalog(catalog_id).pages(pages_tree_id);
    pdf.pages(pages_tree_id).kids([page_id]).count(1);

    let mut page = pdf.page(page_id);
    page.parent(pages_tree_id);
    page.media_box(PdfRect::new(0.0, 0.0, 300.0, 150.0));
    page.contents(content_id);
    page.resources().fonts().pair(Name(b"F1"), type0_font_id);
    page.finish();

    let mut content = Content::new();
    content.begin_text();
    content.set_font(Name(b"F1"), font_size);
    content.next_line(20.0, 90.0);
    content.show(Str(&glyph_bytes));
    content.end_text();
    pdf.stream(content_id, &content.finish());

    // フォントプログラム本体を埋め込む(TrueType、無圧縮)。
    let font_data = font.data();
    let mut font_file = pdf.stream(font_file_id, font_data);
    font_file.pair(Name(b"Length1"), font_data.len() as i32);
    font_file.finish();

    let bbox = font.bounding_box();
    pdf.font_descriptor(descriptor_id)
        .name(Name(b"DejaVuSans"))
        .flags(pdf_writer::types::FontFlags::NON_SYMBOLIC)
        .bbox(PdfRect::new(
            to_1000(bbox.x_min as f32),
            to_1000(bbox.y_min as f32),
            to_1000(bbox.x_max as f32),
            to_1000(bbox.y_max as f32),
        ))
        .italic_angle(font.italic_angle())
        .ascent(to_1000(font.ascender() as f32))
        .descent(to_1000(font.descender() as f32))
        .cap_height(to_1000(
            font.capital_height().unwrap_or(font.ascender()) as f32
        ))
        .stem_v(if font.weight() >= 700 { 120.0 } else { 80.0 })
        .font_file2(font_file_id);

    let mut cid_font = pdf.cid_font(cid_font_id);
    cid_font.subtype(pdf_writer::types::CidFontType::Type2);
    cid_font.base_font(Name(b"DejaVuSans"));
    cid_font.system_info(pdf_writer::types::SystemInfo {
        registry: Str(b"Adobe"),
        ordering: Str(b"Identity"),
        supplement: 0,
    });
    cid_font.font_descriptor(descriptor_id);
    cid_font.default_width(0.0);
    {
        let mut w = cid_font.widths();
        for (&gid, &width) in &widths {
            w.same(gid, gid, width);
        }
        w.finish();
    }
    cid_font.cid_to_gid_map_predefined(Name(b"Identity"));
    cid_font.finish();

    pdf.type0_font(type0_font_id)
        .base_font(Name(b"DejaVuSans"))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(cid_font_id);

    let bytes = pdf.finish();

    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/spike_pdf_font_embedding.pdf");
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {} bytes to {}", bytes.len(), out.display());
}
