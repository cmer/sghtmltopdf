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
//! - Ruby側のFFI境界(`Engine.new(options)` / `feed(html_chunk)` /
//!   `each_pdf_chunk { |bytes| ... }` / `finish`)に、コア側のRust APIを
//!   ほぼ1:1で対応させられるか
//! - Rubyの`each_pdf_chunk { |bytes| ... }`ブロックを、コアに手を入れずに
//!   「呼ばれるたびにブロックを呼ぶだけのSink実装」でラップできるか
//!   (`CallbackSink`で模擬)
//! - `Mode::Batch`/`Mode::Streaming`の切り替えが同じ`Engine`型に
//!   自然に載るか。Batchモードは非局所性の制約を一切課さず(DOM全体を
//!   待ってから処理するため`nth-last-child`等も問題なく扱える)、
//!   Streamingモードのみ制約を適用する(body途中の`<style>`はエラーで
//!   拒否する)
//!
//! 実行: `cargo run --example spike_streaming_engine_api`

use sghtmltopdf_core::sink::{MemorySink, Sink};

/// 一括処理かストリーミング処理かを選択する。
///
/// `Batch`はDOM全体が揃ってから処理するため、非局所性の制約(`nth-last-child`
/// 等の非サポート、`<style>`は`<head>`内のみ)を一切課さない。
/// `Streaming`のみこれらの制約を適用する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Batch,
    Streaming,
}

/// エンジンの初期化オプション(ページサイズ・マージン等)のプレースホルダ。
/// 実際のフィールドはT21以降、CLI/bindings層のオプションと揃えて決める。
#[derive(Default)]
struct EngineOptions {
    mode: Mode,
}

/// `Engine`が返すエラー。Sinkへの書き込み失敗(`Io`)と、Streamingモードで
/// 検出したサポート外のHTML構造(`UnsupportedInStreamingMode`)を区別する。
/// 後者は`Sink::Error`と無関係な、コア自身が判定するエラーのため
/// `S::Error`にそのまま相乗りさせず専用バリアントを設ける。
#[derive(Debug)]
enum EngineError<E> {
    Io(E),
    UnsupportedInStreamingMode(&'static str),
}

impl<E> From<E> for EngineError<E> {
    fn from(e: E) -> Self {
        Self::Io(e)
    }
}

/// flush boundary(ページ確定)ごとに`sink.write`を呼びながらHTMLを消費する
/// ストリーミングエンジンの型シグネチャ。中身はT21〜T25で埋める。
struct Engine<S: Sink> {
    sink: S,
    options: EngineOptions,
    /// ダミー実装用: feedで受け取ったチャンクを溜めておくだけ。
    /// 本実装ではストリーミングパーサ+レイアウトの内部状態に置き換わる。
    pending: Vec<u8>,
    /// ダミー実装用: `<body`を見たかどうか(本実装ではTreeSinkのフックで
    /// 判定する。T21のスコープ)。
    seen_body: bool,
}

impl<S: Sink> Engine<S> {
    fn new(options: EngineOptions, sink: S) -> Self {
        Self {
            sink,
            options,
            pending: Vec::new(),
            seen_body: false,
        }
    }

    /// HTMLチャンクを1つ投入する。内部で新たにflush boundary(ページ確定)に
    /// 到達した分があれば、そのつど`sink.write`を呼ぶ(このダミー実装では
    /// 呼ばない。T23でレイアウトのflush化と合わせて実装する)。
    ///
    /// `Mode::Streaming`では、`<body`より後に`<style`が現れたらエラーを返す。
    /// 実際の判定はT21でTreeSinkのフックとして実装する(ここでは検証用にバイト
    /// 列の雑な走査で代用している)。
    fn feed(&mut self, html_chunk: &[u8]) -> Result<(), EngineError<S::Error>> {
        if self.options.mode == Mode::Streaming {
            if !self.seen_body && contains(html_chunk, b"<body") {
                self.seen_body = true;
            }
            if self.seen_body && contains(html_chunk, b"<style") {
                return Err(EngineError::UnsupportedInStreamingMode(
                    "<style> after <body> is not supported in streaming mode",
                ));
            }
        }
        self.pending.extend_from_slice(html_chunk);
        Ok(())
    }

    /// 残りの内容を最後のページとして確定させ、フォント埋め込み等の
    /// 全ページ後処理(T18のCIDToGIDMap方式、T25で本実装)を行ってから
    /// `sink.finish()`を呼ぶ。
    fn finish(mut self) -> Result<S::Output, EngineError<S::Error>> {
        // ダミー実装: 溜めたチャンクをそのまま1回書き出すだけ。
        // 本実装ではここでPDFバイト列(コンテンツストリーム+フォント埋め込み)
        // を組み立てて書く。
        self.sink.write(&self.pending)?;
        Ok(self.sink.finish()?)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
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
    // --- 同期返却モード相当: MemorySinkへ書き込む(既定はBatch) ---
    let mut engine = Engine::new(EngineOptions::default(), MemorySink::new());
    engine.feed(b"<p>Hello").unwrap();
    engine.feed(b", world!</p>").unwrap();
    let bytes = engine.finish().unwrap();
    eprintln!(
        "MemorySink経由(Batch): {} bytes -> {:?}",
        bytes.len(),
        String::from_utf8_lossy(&bytes)
    );

    // --- each_pdf_chunk { |bytes| ... } 相当: コールバックSinkへ書き込む ---
    let mut chunks_seen = Vec::new();
    let callback_sink = CallbackSink {
        callback: |bytes: &[u8]| chunks_seen.push(bytes.to_vec()),
    };
    let mut engine = Engine::new(EngineOptions::default(), callback_sink);
    engine.feed(b"<p>chunked").unwrap();
    engine.feed(b" input</p>").unwrap();
    engine.finish().unwrap();
    eprintln!(
        "CallbackSink経由(Batch): {}回書き込みを観測",
        chunks_seen.len()
    );

    // --- Batchモード: body途中の<style>があっても許容される ---
    let mut engine = Engine::new(EngineOptions { mode: Mode::Batch }, MemorySink::new());
    engine.feed(b"<body><p>x</p>").unwrap();
    engine
        .feed(b"<style>p{color:red}</style>")
        .expect("Batchモードではbody途中の<style>もエラーにならないはず");
    engine.finish().unwrap();
    eprintln!("Batchモード: body途中の<style>を許容");

    // --- Streamingモード: body途中の<style>はエラーになる ---
    let mut engine = Engine::new(
        EngineOptions {
            mode: Mode::Streaming,
        },
        MemorySink::new(),
    );
    engine.feed(b"<body><p>x</p>").unwrap();
    match engine.feed(b"<style>p{color:red}</style>") {
        Err(EngineError::UnsupportedInStreamingMode(msg)) => {
            eprintln!("Streamingモード: 期待通りエラーを検出 ({msg})");
        }
        other => panic!("expected UnsupportedInStreamingMode, got {other:?}"),
    }
}
