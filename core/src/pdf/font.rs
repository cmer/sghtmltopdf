//! フォントのPDF埋め込み(CIDFontType2 + Type0 `/Encoding /Identity-H`)。
//!
//! `core/examples/spike_pdf_font_embedding.rs`で検証した方式をベースに、
//! 実際に使用したグリフだけへのサブセット化(`subsetter`クレート)と、
//! `/ToUnicode` CMapによるテキスト抽出対応を追加している。
//!
//! `subsetter::subset`はサブセット後のフォントから`cmap`テーブルを取り除く仕様
//! (PDF埋め込み専用の割り切った設計)のため、サブセット後のグリフIDは元の
//! グリフIDとは異なる(コンパクトに詰め直された)ものになる。そのため
//! [`embed_font`]は「元のグリフID→サブセット後のグリフID(=CID)」の対応表を
//! 返し、呼び出し側([`super::document`])はコンテンツストリームを書く際に
//! この対応表でグリフIDを変換する必要がある。
//!
//! 既知の未対応事項(将来のマイルストーンで対応):
//! - フォントストリームは無圧縮のまま埋め込む(`flate2`等の追加が必要)

use std::collections::BTreeMap;

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap};
use pdf_writer::{Finish, Name, Pdf, Rect as PdfRect, Ref, Str};
use subsetter::GlyphRemapper;

use crate::fonts::Font;

/// 埋め込むフォント一式のオブジェクトID。
#[derive(Debug, Clone, Copy)]
pub struct FontIds {
    pub font_file: Ref,
    pub descriptor: Ref,
    pub cid_font: Ref,
    pub type0_font: Ref,
    pub to_unicode: Ref,
}

/// 1フォント分の使用状況(文書全体を1パス目で走査して集める)。
#[derive(Debug, Default)]
pub struct FontUsage {
    /// 元のグリフID -> (幅[1000unit/emグリフ空間], 代表Unicode文字)。
    glyphs: BTreeMap<u16, (f32, char)>,
}

impl FontUsage {
    /// `glyph_id`の使用を記録する。`unicode`は`/ToUnicode`生成用の代表文字
    /// (`ShapedGlyph::cluster`から元テキストを逆引きしたもの)。
    pub fn record(&mut self, font: &Font, glyph_id: u16, unicode: char) {
        self.glyphs.entry(glyph_id).or_insert_with(|| {
            let advance = font.glyph_hor_advance(glyph_id).unwrap_or(0) as f32;
            let width_1000 = advance * 1000.0 / font.units_per_em() as f32;
            (width_1000, unicode)
        });
    }
}

/// `font`をPDFへ埋め込む(`usage`に記録されたグリフだけにサブセット化する)。
///
/// 返り値は「元のグリフID→サブセット後のグリフID(CID)」の対応表。
pub fn embed_font(
    pdf: &mut Pdf,
    font: &Font,
    ids: FontIds,
    usage: &FontUsage,
) -> BTreeMap<u16, u16> {
    let mut remapper = GlyphRemapper::new();
    remapper.remap(0); // .notdef
    for &old_gid in usage.glyphs.keys() {
        remapper.remap(old_gid);
    }

    let subset_data = subsetter::subset(font.data(), font.face_index(), &remapper)
        .unwrap_or_else(|_| font.data().to_vec());

    let mut font_file = pdf.stream(ids.font_file, &subset_data);
    font_file.pair(Name(b"Length1"), subset_data.len() as i32);
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

    let old_to_new: BTreeMap<u16, u16> = usage
        .glyphs
        .keys()
        .map(|&old_gid| {
            let new_gid = remapper
                .get(old_gid)
                .expect("usageに記録済みのグリフは必ずremapされている");
            (old_gid, new_gid)
        })
        .collect();

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
        for (&old_gid, &(width, _)) in &usage.glyphs {
            let new_gid = old_to_new[&old_gid];
            w.same(new_gid, new_gid, width);
        }
        w.finish();
    }
    cid_font.cid_to_gid_map_predefined(Name(b"Identity"));
    cid_font.finish();

    let mut cmap = UnicodeCmap::<u16>::new(
        Name(b"Custom"),
        SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"UCS"),
            supplement: 0,
        },
    );
    for (&old_gid, &(_, unicode)) in &usage.glyphs {
        cmap.pair(old_to_new[&old_gid], unicode);
    }
    let cmap_bytes = cmap.finish();
    pdf.cmap(ids.to_unicode, &cmap_bytes).finish();

    pdf.type0_font(ids.type0_font)
        .base_font(Name(b"EmbeddedFont"))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(ids.cid_font)
        .to_unicode(ids.to_unicode);

    old_to_new
}
