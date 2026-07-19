//! T18スパイク: フォントサブセット化とページ単位のストリーミング出力を両立できるか検証するPoC。
//!
//! 現状の`pdf::document::encode_pdf`は「Pass 1で全ページを走査してグリフ使用状況を
//! 集計 → サブセット化(グリフIDを詰め直す)→ Pass 2でコンテンツストリームを書く」
//! という2パス構成で、コンテンツストリーム自体がサブセット結果(元GID→CID変換表)に
//! 依存している。これは全ページ走査が終わるまでどのページのコンテンツストリームも
//! 書けないことを意味し、「ページ確定のそばから逐次書き出す」ストリーミング出力と
//! 相容れない。
//!
//! ここで検証するのは、次の設計でこの依存を切れるかどうか:
//! - コンテンツストリームでは常に**元のグリフID**をそのままCIDとして使う
//!   (`/Encoding /Identity-H`のまま、CIDを詰め直さない)。これによりページの
//!   コンテンツストリームは、そのページのシェイピングが終わった時点で(=他ページの
//!   状況を待たずに)即座に確定・書き出しできる
//! - フォント埋め込み(`/FontFile2`本体)は従来通りサブセット化する。ただし
//!   `/CIDToGIDMap`を`/Identity`ではなく、CID(=元GID)→サブセット後GIDの対応表を
//!   持つ明示的なストリーム(`cid_to_gid_map_stream`)にすることで、コンテンツ
//!   ストリーム側のCID(元GID)とサブセット後のフォントの整合を取る
//!
//! この方式なら、ページ確定ごとに保持し続ける必要があるのは軽量な`FontUsage`
//! (グリフIDと幅・代表Unicode文字の集計)のみになり、レイアウト結果やコンテンツ
//! ストリームのバイト列自体は都度破棄できる。フォント埋め込みオブジェクト自体は
//! 従来通り全ページ処理後に1回だけ書く(Chunkとして最後に追記する)。
//!
//! 実行: `cargo run --example spike_streaming_font_subset`
//! 検証: `python3 -c "import fitz; d=fitz.open('target/spike_streaming_font_subset.pdf'); \
//!   print([p.get_text() for p in d])"` でテキスト抽出、目視でグリフも確認する。

use std::collections::BTreeMap;

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::writers::Catalog;
use pdf_writer::{Chunk, Content, Filter, Finish, Name, Rect as PdfRect, Ref, Str};
use sghtmltopdf_core::fonts::{shape_text, Font};
use subsetter::GlyphRemapper;

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

/// ページ確定ごとに即座にバイト列を書き出す疑似Sink(spike_pdf_writer.rsと同様)。
struct FakeSink {
    output: Vec<u8>,
    offsets: Vec<(Ref, usize)>,
}

impl FakeSink {
    fn new() -> Self {
        let output = b"%PDF-1.7\n%\x80\x80\x80\x80\n\n".to_vec();
        Self {
            output,
            offsets: Vec::new(),
        }
    }

    fn write_chunk(&mut self, id: Ref, chunk: &Chunk) {
        self.offsets.push((id, self.output.len()));
        self.output.extend_from_slice(chunk.as_bytes());
    }

    fn finish(mut self, root: Ref) -> Vec<u8> {
        let xref_offset = self.output.len();
        let size = self
            .offsets
            .iter()
            .map(|(id, _)| id.get())
            .max()
            .unwrap_or(0)
            + 1;

        self.offsets.sort_by_key(|(id, _)| id.get());
        self.output
            .extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        self.output.extend_from_slice(b"0000000000 65535 f \n");
        for (_, offset) in &self.offsets {
            self.output
                .extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }

        self.output.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root {} 0 R >>\n", root.get()).as_bytes(),
        );
        self.output
            .extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF").as_bytes());
        self.output
    }
}

/// 文書全体で使われたグリフの軽量な集計(実際のコンテンツストリームは保持しない)。
#[derive(Default)]
struct FontUsage {
    /// 元のグリフID -> (幅[1000unit/emグリフ空間], 代表Unicode文字)。
    glyphs: BTreeMap<u16, (f32, char)>,
}

fn main() {
    let font = Font::load(FONT_PATH).expect("should load bundled test font");
    let units_per_em = font.units_per_em() as f32;
    let to_1000 = |font_units: f32| font_units * 1000.0 / units_per_em;

    // 2ページ分、それぞれ異なるテキスト(=異なるグリフ集合)を用意する。
    // ページ2はページ1と一部グリフが重複しつつ、ページ1には出てこない文字("Q", "Z"等)も含む。
    let page_texts = ["Hello, world!", "Quick zebra jumps."];

    let mut ids = 0..;
    let mut next_id = || Ref::new(ids.next().unwrap() + 1);

    let catalog_id = next_id();
    let pages_tree_id = next_id();
    let font_file_id = next_id();
    let descriptor_id = next_id();
    let cid_font_id = next_id();
    let type0_font_id = next_id();
    let cid_to_gid_id = next_id();

    let mut sink = FakeSink::new();
    let mut usage = FontUsage::default();
    let mut page_ids = Vec::new();

    // --- ページごとの逐次処理: 他ページの状況を一切待たず、シェイピング直後に
    //     コンテンツストリームをChunkとして確定・書き出す ---
    for text in page_texts {
        let page_id = next_id();
        let content_id = next_id();
        page_ids.push(page_id);

        let shaped = shape_text(&font, text, 24.0);

        // CIDはリマップせず、常に元のグリフIDをそのまま使う。
        let mut glyph_bytes = Vec::with_capacity(shaped.glyphs.len() * 2);
        for g in &shaped.glyphs {
            glyph_bytes.extend_from_slice(&g.glyph_id.to_be_bytes());
            let unicode = text[g.cluster as usize..].chars().next().unwrap_or('?');
            usage.glyphs.entry(g.glyph_id).or_insert_with(|| {
                let advance = font.glyph_hor_advance(g.glyph_id).unwrap_or(0) as f32;
                (to_1000(advance), unicode)
            });
        }

        let mut content = Content::new();
        content.begin_text();
        content.set_font(Name(b"F1"), 24.0);
        content.next_line(20.0, 90.0);
        content.show(Str(&glyph_bytes));
        content.end_text();

        let mut chunk = Chunk::new();
        chunk.stream(content_id, &content.finish());
        sink.write_chunk(content_id, &chunk);

        let mut chunk = Chunk::new();
        chunk
            .page(page_id)
            .parent(pages_tree_id)
            .media_box(PdfRect::new(0.0, 0.0, 300.0, 150.0))
            .contents(content_id)
            .resources()
            .fonts()
            .pair(Name(b"F1"), type0_font_id);
        sink.write_chunk(page_id, &chunk);
    }

    // --- ここから先は全ページ処理後の1回きりの後処理。保持していたのは
    //     軽量な`usage`(グリフID集合)のみで、コンテンツストリームの生データや
    //     レイアウト結果は一切保持していない ---

    let mut remapper = GlyphRemapper::new();
    remapper.remap(0); // .notdef
    for &old_gid in usage.glyphs.keys() {
        remapper.remap(old_gid);
    }
    let subset_data = subsetter::subset(font.data(), font.face_index(), &remapper)
        .expect("subsetting should succeed for the bundled test font");

    // CIDToGIDMap: CID(=元GID)でインデックスした2バイトのGID値のテーブル。
    // 未使用のCIDは0(.notdef)のままにする。
    let max_gid = usage.glyphs.keys().copied().max().unwrap_or(0);
    let mut cid_to_gid_bytes = vec![0u8; (max_gid as usize + 1) * 2];
    for &old_gid in usage.glyphs.keys() {
        let new_gid = remapper
            .get(old_gid)
            .expect("usageに記録済みのグリフは必ずremapされている");
        let idx = old_gid as usize * 2;
        cid_to_gid_bytes[idx..idx + 2].copy_from_slice(&new_gid.to_be_bytes());
    }

    let compressed_cid_to_gid = deflate(&cid_to_gid_bytes);
    let mut chunk = Chunk::new();
    let mut cid_to_gid_stream = chunk.stream(cid_to_gid_id, &compressed_cid_to_gid);
    cid_to_gid_stream.filter(Filter::FlateDecode);
    cid_to_gid_stream.finish();
    sink.write_chunk(cid_to_gid_id, &chunk);

    let compressed_font = deflate(&subset_data);
    let mut chunk = Chunk::new();
    let mut font_file = chunk.stream(font_file_id, &compressed_font);
    font_file.filter(Filter::FlateDecode);
    font_file.pair(Name(b"Length1"), subset_data.len() as i32);
    font_file.finish();
    sink.write_chunk(font_file_id, &chunk);

    let bbox = font.bounding_box();
    let mut chunk = Chunk::new();
    chunk
        .font_descriptor(descriptor_id)
        .name(Name(b"DejaVuSans"))
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
        .stem_v(80.0)
        .font_file2(font_file_id);
    sink.write_chunk(descriptor_id, &chunk);

    let mut chunk = Chunk::new();
    let mut cid_font = chunk.cid_font(cid_font_id);
    cid_font.subtype(CidFontType::Type2);
    cid_font.base_font(Name(b"DejaVuSans"));
    cid_font.system_info(SystemInfo {
        registry: Str(b"Adobe"),
        ordering: Str(b"Identity"),
        supplement: 0,
    });
    cid_font.font_descriptor(descriptor_id);
    cid_font.default_width(0.0);
    {
        // /Wは元のグリフID(=CID)をキーに、サブセット前と同じ値をそのまま書ける
        // (幅はusage収集時点で元GIDベースに記録済みのため変換不要)。
        let mut w = cid_font.widths();
        for (&old_gid, &(width, _)) in &usage.glyphs {
            w.same(old_gid, old_gid, width);
        }
        w.finish();
    }
    // Identityではなく、サブセット後の実グリフ位置への明示マップを使う。
    cid_font.cid_to_gid_map_stream(cid_to_gid_id);
    cid_font.finish();
    sink.write_chunk(cid_font_id, &chunk);

    let mut chunk = Chunk::new();
    chunk
        .type0_font(type0_font_id)
        .base_font(Name(b"DejaVuSans"))
        .encoding_predefined(Name(b"Identity-H"))
        .descendant_font(cid_font_id);
    sink.write_chunk(type0_font_id, &chunk);

    let mut chunk = Chunk::new();
    chunk
        .pages(pages_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);
    sink.write_chunk(pages_tree_id, &chunk);

    let mut chunk = Chunk::new();
    chunk
        .indirect(catalog_id)
        .start::<Catalog>()
        .pages(pages_tree_id);
    sink.write_chunk(catalog_id, &chunk);

    let pdf = sink.finish(catalog_id);

    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/spike_streaming_font_subset.pdf");
    std::fs::write(&out, &pdf).unwrap();
    eprintln!(
        "wrote {} bytes to {} (max_gid={max_gid}, used_glyphs={})",
        pdf.len(),
        out.display(),
        usage.glyphs.len()
    );
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}
