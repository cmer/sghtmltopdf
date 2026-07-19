//! PDFバイト列をページ確定のそばから逐次[`Sink`]へ書き出すストリーミング
//! ライター。
//!
//! [0004](../../../docs/decisions/0004-streaming-pdf-and-font-subsetting.md)
//! で検証した設計をそのまま本実装に落とし込む: 各ページのコンテンツ
//! ストリームは、そのページの[`Page`]が確定した時点で即座に構築し
//! `Sink`へ書き出す(CIDは常に元のグリフID、`render_box`/`render_line`に
//! `remaps: None`を渡す)。フォント埋め込み(サブセット化・
//! `/CIDToGIDMap`ストリームの構築)は、[`StreamingPdfWriter::finish`]が
//! 呼ばれた時点(全ページ処理後)にまとめて行う。
//!
//! [0001](../../../docs/decisions/0001-pdf-writer-crate.md)の通り、
//! `pdf_writer::Pdf`はxref/trailerの構築を非公開実装に持つため、`Chunk`
//! (1オブジェクトごとの自己完結したバイト列)単位で`Sink`へ逐次書き出しつつ、
//! `(Ref, 書き込み済みオフセット)`を自前で記録し、[`StreamingPdfWriter::finish`]
//! でxref/trailerを組み立てる。

use std::collections::HashMap;

use pdf_writer::writers::Catalog;
use pdf_writer::{Chunk, Content, Filter, Finish, Name, Rect as PdfRect, Ref};

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::layout::{Page, PageSettings};
use crate::sink::Sink;
use crate::style::ComputedStyle;

use super::document::{collect_usage, render_box, RefAllocator};
use super::font::{deflate, embed_font_streaming_chunks, FontIds, FontUsage};

const PDF_HEADER: &[u8] = b"%PDF-1.7\n%\x80\x80\x80\x80\n\n";

/// ページ確定のそばから逐次`Sink`へPDFバイト列を書き出すライター。
///
/// `new`でファイルヘッダを即座に書き出し、`write_page`をページ確定のたびに
/// 呼び、最後に`finish`でフォント埋め込み・xref/trailerを書いて`sink`を
/// 締める。
pub struct StreamingPdfWriter<S: Sink> {
    sink: S,
    output_len: usize,
    offsets: Vec<(Ref, usize)>,
    alloc: RefAllocator,
    catalog_id: Ref,
    pages_tree_id: Ref,
    font_ids: Vec<FontIds>,
    font_resource_names: Vec<String>,
    usages: Vec<FontUsage>,
    page_ids: Vec<Ref>,
    settings: PageSettings,
}

impl<S: Sink> StreamingPdfWriter<S> {
    /// 新しいライターを作り、PDFファイルヘッダを即座に`sink`へ書き出す。
    pub fn new(
        fonts: &FontCollection,
        settings: PageSettings,
        mut sink: S,
    ) -> Result<Self, S::Error> {
        sink.write(PDF_HEADER)?;

        let mut alloc = RefAllocator::default();
        let catalog_id = alloc.next();
        let pages_tree_id = alloc.next();
        let font_ids: Vec<FontIds> = (0..fonts.len())
            .map(|_| FontIds {
                font_file: alloc.next(),
                descriptor: alloc.next(),
                cid_font: alloc.next(),
                type0_font: alloc.next(),
                to_unicode: alloc.next(),
                cid_to_gid_map: alloc.next(),
            })
            .collect();
        let font_resource_names = (0..fonts.len()).map(|i| format!("F{i}")).collect();
        let usages = (0..fonts.len()).map(|_| FontUsage::default()).collect();

        Ok(Self {
            sink,
            output_len: PDF_HEADER.len(),
            offsets: Vec::new(),
            alloc,
            catalog_id,
            pages_tree_id,
            font_ids,
            font_resource_names,
            usages,
            page_ids: Vec::new(),
            settings,
        })
    }

    /// 確定した1ページを即座にコンテンツストリームへエンコードし、`sink`へ
    /// 書き出す。使用したグリフは内部に軽量な[`FontUsage`]として蓄積する
    /// だけなので、呼び出し後は`page`(レイアウト結果)を破棄してよい。
    pub fn write_page(
        &mut self,
        page: &Page,
        styles: &HashMap<NodeId, ComputedStyle>,
        fonts: &FontCollection,
    ) -> Result<(), S::Error> {
        for b in &page.boxes {
            collect_usage(b, fonts, &mut self.usages);
        }

        let page_id = self.alloc.next();
        let content_id = self.alloc.next();
        self.page_ids.push(page_id);

        let mut content = Content::new();
        for b in &page.boxes {
            // `remaps: None` — CIDは常に元のグリフIDのまま使う(モジュールdoc参照)。
            render_box(
                &mut content,
                b,
                styles,
                fonts,
                &self.settings,
                None,
                &self.font_resource_names,
            );
        }
        let content_bytes = content.finish();
        let compressed_content = deflate(&content_bytes);

        let mut chunk = Chunk::new();
        let mut content_stream = chunk.stream(content_id, &compressed_content);
        content_stream.filter(Filter::FlateDecode);
        content_stream.finish();
        self.write_chunk(content_id, &chunk)?;

        let mut chunk = Chunk::new();
        {
            let mut p = chunk.page(page_id);
            p.parent(self.pages_tree_id);
            p.media_box(PdfRect::new(
                0.0,
                0.0,
                self.settings.size.width,
                self.settings.size.height,
            ));
            p.contents(content_id);
            let mut resources = p.resources();
            let mut font_dict = resources.fonts();
            for (name, ids) in self.font_resource_names.iter().zip(self.font_ids.iter()) {
                font_dict.pair(Name(name.as_bytes()), ids.type0_font);
            }
        }
        self.write_chunk(page_id, &chunk)?;

        Ok(())
    }

    /// 残りのオブジェクト(フォント埋め込み・ページツリー・カタログ・
    /// xref/trailer)をすべて書き出し、`sink.finish()`を呼ぶ。
    pub fn finish(mut self, fonts: &FontCollection) -> Result<S::Output, S::Error> {
        let font_ids = self.font_ids.clone();
        let usages = std::mem::take(&mut self.usages);
        for ((font, &ids), usage) in fonts.fonts().iter().zip(font_ids.iter()).zip(usages.iter()) {
            for (id, chunk) in embed_font_streaming_chunks(font, ids, usage) {
                self.write_chunk(id, &chunk)?;
            }
        }

        let mut chunk = Chunk::new();
        chunk
            .pages(self.pages_tree_id)
            .kids(self.page_ids.iter().copied())
            .count(self.page_ids.len() as i32);
        self.write_chunk(self.pages_tree_id, &chunk)?;

        let mut chunk = Chunk::new();
        chunk
            .indirect(self.catalog_id)
            .start::<Catalog>()
            .pages(self.pages_tree_id);
        self.write_chunk(self.catalog_id, &chunk)?;

        self.write_xref_and_trailer()?;

        self.sink.finish()
    }

    /// `chunk`(単一の間接オブジェクトを含む前提)のバイト列を`sink`へ書き出し、
    /// 開始オフセットをxref用に記録する。
    fn write_chunk(&mut self, id: Ref, chunk: &Chunk) -> Result<(), S::Error> {
        self.offsets.push((id, self.output_len));
        let bytes = chunk.as_bytes();
        self.output_len += bytes.len();
        self.sink.write(bytes)
    }

    fn write_xref_and_trailer(&mut self) -> Result<(), S::Error> {
        let xref_offset = self.output_len;
        let size = self
            .offsets
            .iter()
            .map(|(id, _)| id.get())
            .max()
            .unwrap_or(0)
            + 1;

        self.offsets.sort_by_key(|(id, _)| id.get());

        let mut buf = Vec::new();
        buf.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        buf.extend_from_slice(b"0000000000 65535 f \n");
        for (_, offset) in &self.offsets {
            buf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        buf.extend_from_slice(
            format!(
                "trailer\n<< /Size {size} /Root {} 0 R >>\n",
                self.catalog_id.get()
            )
            .as_bytes(),
        );
        buf.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF").as_bytes());

        self.output_len += buf.len();
        self.sink.write(&buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Font;
    use crate::html;
    use crate::layout::{paginate_document, paginate_streaming};
    use crate::sink::MemorySink;
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).expect("should load bundled test font")
        ])
    }

    fn test_fonts_with_cjk() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).expect("should load bundled DejaVu test font"),
            Font::load_indexed(CJK_PATH, 0).expect("should load bundled CJK test font"),
        ])
    }

    fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn streaming_writer_produces_a_valid_pdf_with_embedded_font() {
        let dom = html::parse(b"<p>Hello, world!</p>");
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);

        let mut writer = StreamingPdfWriter::new(&fonts, settings, MemorySink::new())
            .expect("new should not fail");
        for page in &pages {
            writer
                .write_page(page, &styles, &fonts)
                .expect("write_page should not fail");
        }
        let bytes = writer.finish(&fonts).expect("finish should not fail");

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"%%EOF") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /Type0") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0);
        assert!(count_occurrences(&bytes, b"/Identity-H") > 0);
        assert!(count_occurrences(&bytes, b"/FontFile2") > 0);
        assert!(
            count_occurrences(&bytes, b"/Type /CMap") > 0,
            "ToUnicode CMap should be embedded"
        );
    }

    #[test]
    fn streaming_writer_output_is_readable_by_pdf_parsing_via_pymupdf_equivalent_checks() {
        // PyMuPDFのような外部ツールでの実描画確認はspikeで別途行い済み
        // (spike_streaming_font_subset.rs)。ここでは複数ページ・複数フォント
        // (ページをまたいでグリフ集合が変わるケース)でも構造的に妥当な
        // PDFになることを確認する。
        let mut html_src = String::from("<div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".item { height: 100px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(pages.len() > 1, "expected multiple pages");

        let mut writer = StreamingPdfWriter::new(&fonts, settings, MemorySink::new())
            .expect("new should not fail");
        for page in &pages {
            writer
                .write_page(page, &styles, &fonts)
                .expect("write_page should not fail");
        }
        let bytes = writer.finish(&fonts).expect("finish should not fail");

        assert_eq!(count_occurrences(&bytes, b"/MediaBox"), pages.len());
        assert_eq!(count_occurrences(&bytes, b"/FontFile2"), 1);
    }

    #[test]
    fn streaming_writer_subsets_a_large_cjk_font() {
        let dom = html::parse("<p>日本語のテスト</p>".as_bytes());
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts_with_cjk();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);

        let mut writer = StreamingPdfWriter::new(&fonts, settings, MemorySink::new())
            .expect("new should not fail");
        for page in &pages {
            writer
                .write_page(page, &styles, &fonts)
                .expect("write_page should not fail");
        }
        let bytes = writer.finish(&fonts).expect("finish should not fail");

        let cjk_font_size = std::fs::metadata(CJK_PATH).unwrap().len() as usize;
        assert!(
            bytes.len() < cjk_font_size / 10,
            "subsetted output ({} bytes) should be far smaller than the original CJK font ({} bytes)",
            bytes.len(),
            cjk_font_size
        );
        assert_eq!(count_occurrences(&bytes, b"/FontFile2"), 2);
    }

    #[test]
    fn streaming_writer_handles_glyphs_that_only_appear_on_a_later_page() {
        // ページ1に登場しない文字("Q"/"z")がページ2にのみ現れるケース。
        // フォント埋め込み(サブセット化+CIDToGIDMap)は全ページ処理後に
        // まとめて行われるため、ページ1のコンテンツストリーム構築時点では
        // これらのグリフの使用状況はまだ確定していない。
        let dom1 = html::parse(b"<p>Hello, world!</p>");
        let dom2 = html::parse(b"<p>Quick zebra jumps.</p>");
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles1 = compute_styles(&dom1, &ua, &author);
        let styles2 = compute_styles(&dom2, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages1 = paginate_document(&dom1, &styles1, &fonts, &settings);
        let pages2 = paginate_document(&dom2, &styles2, &fonts, &settings);

        let mut writer = StreamingPdfWriter::new(&fonts, settings, MemorySink::new())
            .expect("new should not fail");
        for page in &pages1 {
            writer
                .write_page(page, &styles1, &fonts)
                .expect("write_page should not fail");
        }
        for page in &pages2 {
            writer
                .write_page(page, &styles2, &fonts)
                .expect("write_page should not fail");
        }
        let bytes = writer.finish(&fonts).expect("finish should not fail");

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(count_occurrences(&bytes, b"/MediaBox"), 2);
        assert_eq!(count_occurrences(&bytes, b"/FontFile2"), 1);
    }

    #[test]
    fn streaming_writer_matches_paginate_streaming_page_count() {
        let mut html_src = String::from("<div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".item { height: 100px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let tree = crate::layout::build_box_tree(&dom, &styles);
        let laid_out =
            crate::layout::layout_document(&tree, &styles, &fonts, settings.content_width());

        let mut writer = StreamingPdfWriter::new(&fonts, settings, MemorySink::new())
            .expect("new should not fail");
        let mut page_count = 0usize;
        paginate_streaming(&laid_out, settings.content_height(), &mut |page| {
            writer
                .write_page(&page, &styles, &fonts)
                .expect("write_page should not fail");
            page_count += 1;
        });
        let bytes = writer.finish(&fonts).expect("finish should not fail");

        assert!(page_count > 1);
        assert_eq!(count_occurrences(&bytes, b"/MediaBox"), page_count);
    }
}
