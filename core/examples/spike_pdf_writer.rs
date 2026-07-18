//! T1スパイク: pdf-writerで最小PDF(矩形+1行テキスト)を生成するPoC。
//!
//! krillaのspike(`spike_krilla.rs`)と異なり、こちらは`Pdf`型を使わず
//! `Chunk`単位で直接バイト列を組み立て、ページが確定するたびに
//! 疑似Sink(`output: Vec<u8>`。実装では`Sink::write`相当)へ即座に書き出す。
//! 各Chunkのバイト列自体はそのつど破棄でき、以後保持するのは
//! `(Ref, 書き込み済みオフセット)`という軽量なメタ情報のみでよい。
//! これによりページ数が増えても「書き込み済みページの生データ」がメモリに
//! 積み上がらないことを確認する。xref/trailerはpdf-writerの内部実装が
//! 非公開のため自前で組み立てる。
//!
//! 実行: `cargo run --example spike_pdf_writer`(フォント埋め込み不要。base14のHelveticaを使用)

use pdf_writer::writers::Catalog;
use pdf_writer::{Chunk, Content, Name, Rect, Ref, Str};

/// 疑似Sink。書き込みごとにオフセットを記録するだけの最小実装。
struct FakeSink {
    output: Vec<u8>,
    offsets: Vec<(Ref, usize)>,
}

impl FakeSink {
    fn new() -> Self {
        // ファイルヘッダ(`Pdf::new()`が内部で書いているものと同一)。
        // `Chunk`単体にはヘッダが含まれないため自前で先頭に書く。
        let output = b"%PDF-1.7\n%\x80\x80\x80\x80\n\n".to_vec();
        Self {
            output,
            offsets: Vec::new(),
        }
    }

    /// Chunkが単一の間接オブジェクトだけを含む前提で、
    /// そのオブジェクトの開始オフセットを記録しつつバイト列を書き出す。
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

fn main() {
    let mut ids = 0..;
    let mut next_id = || Ref::new(ids.next().unwrap() + 1);

    let catalog_id = next_id();
    let pages_tree_id = next_id();
    let font_id = next_id();

    let mut sink = FakeSink::new();

    // フォント定義(base14のHelvetica。埋め込み不要なのでPoCとしては最小)
    let mut chunk = Chunk::new();
    chunk
        .type1_font(font_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    sink.write_chunk(font_id, &chunk);

    let mut page_ids = Vec::new();
    for page_no in 1..=2 {
        let page_id = next_id();
        let content_id = next_id();
        page_ids.push(page_id);

        // ページの内容ストリーム(矩形+1行テキスト)。
        // レイアウトが確定した時点でこのChunkを組み立て、即座にSinkへ書き出す
        // ── 確定済みページのバイト列をメモリに残さないのが狙い。
        let mut content = Content::new();
        content.set_fill_rgb(0.8, 0.8, 0.8);
        content.rect(20.0, 20.0, 100.0, 50.0);
        content.fill_nonzero();
        content.set_fill_rgb(0.0, 0.0, 0.0);
        content.begin_text();
        content.set_font(Name(b"F1"), 14.0);
        content.next_line(20.0, 100.0);
        content.show(Str(
            format!("Hello from pdf-writer, page {page_no}").as_bytes()
        ));
        content.end_text();
        let content_bytes = content.finish();

        let mut chunk = Chunk::new();
        chunk.stream(content_id, &content_bytes);
        sink.write_chunk(content_id, &chunk);

        let mut chunk = Chunk::new();
        chunk
            .page(page_id)
            .parent(pages_tree_id)
            .media_box(Rect::new(0.0, 0.0, 300.0, 200.0))
            .contents(content_id)
            .resources()
            .fonts()
            .pair(Name(b"F1"), font_id);
        sink.write_chunk(page_id, &chunk);
    }

    // 全ページのRefが出そろって初めて構築できる、ドキュメント全体を跨ぐ小さな部分
    // (ページツリー・カタログ)。ここまで、保持していたのはRefとオフセットのみ。
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

    let out =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/spike_pdf_writer.pdf");
    std::fs::write(&out, &pdf).unwrap();
    eprintln!("wrote {} bytes to {}", pdf.len(), out.display());
}
