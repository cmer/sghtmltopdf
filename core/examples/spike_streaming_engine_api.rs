//! T19スパイク: flush boundaryを表現するコアAPIの型シグネチャが、既存の
//! `Sink` traitと矛盾なく成立するかを確認するPoC。
//!
//! ここではAPIの「形」だけを検証する。ストリーミングパース(T21)・
//! レイアウトのflush化(T23)・フォント埋め込みの後処理(T18で決定済みの
//! CIDToGIDMap方式、T25で本実装)はまだ組み込まず、`feed`/`finish`の中身は
//! ダミー実装にとどめる。
//!
//! 検証したいこと:
//! - `Engine<S: Sink>`がSinkを所有し、`feed`のたびに内部で`sink.write`を
//!   呼べる形が、既存の`Sink`トレイト(`sink/mod.rs`)とそのまま噛み合うか
//! - CLAUDE.mdが示すFFI境界(`Engine.new(options)` / `feed(html_chunk)` /
//!   `each_pdf_chunk { |bytes| ... }` / `finish`)に、コア側のRust APIを
//!   ほぼ1:1で対応させられるか
//! - Rubyの`each_pdf_chunk { |bytes| ... }`ブロックを、コアに手を入れずに
//!   「呼ばれるたびにブロックを呼ぶだけのSink実装」でラップできるか
//!   (`CallbackSink`で模擬)
//!
//! 実行: `cargo run --example spike_streaming_engine_api`

use sghtmltopdf_core::sink::{MemorySink, Sink};

/// エンジンの初期化オプション(ページサイズ・マージン等)のプレースホルダ。
/// 実際のフィールドはT21以降、CLI/bindings層のオプションと揃えて決める。
#[derive(Default)]
struct EngineOptions;

/// flush boundary(ページ確定)ごとに`sink.write`を呼びながらHTMLを消費する
/// ストリーミングエンジンの型シグネチャ。中身はT21〜T25で埋める。
struct Engine<S: Sink> {
    sink: S,
    #[allow(dead_code)]
    options: EngineOptions,
    /// ダミー実装用: feedで受け取ったチャンクを溜めておくだけ。
    /// 本実装ではストリーミングパーサ+レイアウトの内部状態に置き換わる。
    pending: Vec<u8>,
}

impl<S: Sink> Engine<S> {
    fn new(options: EngineOptions, sink: S) -> Self {
        Self {
            sink,
            options,
            pending: Vec::new(),
        }
    }

    /// HTMLチャンクを1つ投入する。内部で新たにflush boundary(ページ確定)に
    /// 到達した分があれば、そのつど`sink.write`を呼ぶ(このダミー実装では
    /// 呼ばない。T23でレイアウトのflush化と合わせて実装する)。
    fn feed(&mut self, html_chunk: &[u8]) -> Result<(), S::Error> {
        self.pending.extend_from_slice(html_chunk);
        Ok(())
    }

    /// 残りの内容を最後のページとして確定させ、フォント埋め込み等の
    /// 全ページ後処理(T18のCIDToGIDMap方式、T25で本実装)を行ってから
    /// `sink.finish()`を呼ぶ。
    fn finish(mut self) -> Result<S::Output, S::Error> {
        // ダミー実装: 溜めたチャンクをそのまま1回書き出すだけ。
        // 本実装ではここでPDFバイト列(コンテンツストリーム+フォント埋め込み)
        // を組み立てて書く。
        self.sink.write(&self.pending)?;
        self.sink.finish()
    }
}

/// Rubyの`each_pdf_chunk { |bytes| ... }`を模した、コールバックを呼ぶだけの
/// Sink実装。コア側に変更を加えず、FFI層だけでこの変換を吸収できることを
/// 確認する。
struct CallbackSink<F: FnMut(&[u8])> {
    callback: F,
}

impl<F: FnMut(&[u8])> Sink for CallbackSink<F> {
    type Output = ();
    type Error = std::io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        (self.callback)(bytes);
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        Ok(())
    }
}

fn main() {
    // --- 同期返却モード相当: MemorySinkへ書き込む ---
    let mut engine = Engine::new(EngineOptions, MemorySink::new());
    engine.feed(b"<p>Hello").unwrap();
    engine.feed(b", world!</p>").unwrap();
    let bytes = engine.finish().unwrap();
    eprintln!(
        "MemorySink経由: {} bytes -> {:?}",
        bytes.len(),
        String::from_utf8_lossy(&bytes)
    );

    // --- each_pdf_chunk { |bytes| ... } 相当: コールバックSinkへ書き込む ---
    let mut chunks_seen = Vec::new();
    let callback_sink = CallbackSink {
        callback: |bytes: &[u8]| chunks_seen.push(bytes.to_vec()),
    };
    let mut engine = Engine::new(EngineOptions, callback_sink);
    engine.feed(b"<p>chunked").unwrap();
    engine.feed(b" input</p>").unwrap();
    engine.finish().unwrap();
    eprintln!("CallbackSink経由: {}回書き込みを観測", chunks_seen.len());
}
