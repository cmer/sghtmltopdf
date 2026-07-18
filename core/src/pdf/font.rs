//! フォントのPDF埋め込み(CIDFontType2 + Type0 `/Encoding /Identity-H`)。
//!
//! `core/examples/spike_pdf_font_embedding.rs`で検証した方式をそのまま使う。
//! 2バイトのコードをそのままCID(=GlyphID、`/CIDToGIDMap /Identity`)として
//! 扱うため、`fonts::shape_text`が返すグリフID列をそのままテキスト描画コードに
//! できる。
//!
//! 既知の未対応事項(将来のマイルストーンで対応):
//! - `/ToUnicode` CMap未設定のため、テキスト抽出/コピペ/全文検索では文字化けする
//!   (表示自体は正しい)
//! - フォントストリームは無圧縮のまま埋め込む(`flate2`等の追加が必要)
//! - フォントサブセット化は未対応(使用したグリフだけに絞り込んでいない)

use std::collections::BTreeMap;

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::{Finish, Name, Pdf, Rect as PdfRect, Ref, Str};

use crate::fonts::Font;

/// 埋め込むフォント一式のオブジェクトID。
#[derive(Debug, Clone, Copy)]
pub struct FontIds {
    pub font_file: Ref,
    pub descriptor: Ref,
    pub cid_font: Ref,
    pub type0_font: Ref,
}

/// `font`をPDFへ埋め込む。`used_glyphs`は文書全体で実際に使用された
/// グリフIDと、その幅(1000unit/emグリフ空間)の対応表。
pub fn embed_font(pdf: &mut Pdf, font: &Font, ids: FontIds, used_glyphs: &BTreeMap<u16, f32>) {
    let font_data = font.data();
    let mut font_file = pdf.stream(ids.font_file, font_data);
    font_file.pair(Name(b"Length1"), font_data.len() as i32);
    font_file.finish();

    let units_per_em = font.units_per_em() as f32;
    let to_1000 = |font_units: f32| font_units * 1000.0 / units_per_em;
    let bbox = font.bounding_box();

    pdf.font_descriptor(ids.descriptor)
        .name(Name(b"EmbeddedFont"))
        .flags(FontFlags::NON_SYMBOLIC)
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
        .font_file2(ids.font_file);

    let mut cid_font = pdf.cid_font(ids.cid_font);
    cid_font.subtype(CidFontType::Type2);
    cid_font.base_font(Name(b"EmbeddedFont"));
    cid_font.system_info(SystemInfo {
        registry: Str(b"Adobe"),
        ordering: Str(b"Identity"),
        supplement: 0,
    });
    cid_font.font_descriptor(ids.descriptor);
    cid_font.default_width(0.0);
    {
        let mut w = cid_font.widths();
        for (&gid, &width) in used_glyphs {
            w.same(gid, gid, width);
        }
        w.finish();
    }
    cid_font.cid_to_gid_map_predefined(Name(b"Identity"));
    cid_font.finish();

    pdf.type0_font(ids.type0_font)
        .base_font(Name(b"EmbeddedFont"))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(ids.cid_font);
}

/// `font_size`で`glyph_id`を描画したときの、1000unit/emグリフ空間での幅を
/// 記録する(まだ記録されていなければ)。
pub fn record_glyph_width(font: &Font, glyph_id: u16, used_glyphs: &mut BTreeMap<u16, f32>) {
    used_glyphs.entry(glyph_id).or_insert_with(|| {
        let advance = font.glyph_hor_advance(glyph_id).unwrap_or(0) as f32;
        advance * 1000.0 / font.units_per_em() as f32
    });
}
