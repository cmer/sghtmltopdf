//! `Engine`: HTMLチャンク投入からPDFバイト列書き出しまでを1つのAPIとして
//! 統合するコアのエントリポイント。
//!
//! [0005](../docs/decisions/0005-engine-streaming-api.md)で決めたSinkベースの
//! `new`/`feed`/`finish`という粗粒度APIを実装する。CLAUDE.mdのFFI境界
//! (`Engine.new(options)` / `feed(html_chunk)` /
//! `each_pdf_chunk { |bytes| ... }` / `finish`)にほぼ1:1で対応する。
//!
//! `Mode::Batch`/`Mode::Streaming`の使い分けは
//! [0006](../docs/decisions/0006-css-non-locality-scope.md)参照。
//!
//! ## 現状の統合範囲(既知の限界)
//!
//! `feed`は[`crate::html::StreamingParser`]へチャンクを逐次投入するが、
//! `finish`が呼ばれるまでスタイル計算(`compute_styles`)・ボックスツリー
//! 構築・レイアウトは一切開始しない(いずれもDOM全体を一括で読む既存実装
//! のまま)。ページ確定ごとのDOM解放・PDFチャンク書き出し(出力側)は
//! `finish`の中で実際にストリーミング処理される([`crate::layout::paginate_document_streaming`]・
//! [`crate::pdf::StreamingPdfWriter`])が、「HTMLを読みながら並行して
//! レイアウトも進める」という意味での入力側の完全なストリーミングは
//! まだ実現できていない。`Mode::Streaming`が実際に強制するのは
//! 「`<body>`より後の`<style>`タグをエラーにする」ことのみで、
//! `nth-last-child`等の非局所セレクタ([0006]の分類3)は、スタイル計算が
//! 依然としてDOM全体一括処理のままであるため、現状では実際には動作して
//! しまう(`Mode::Streaming`を選んでもこの制約はまだ強制されない)。
//! これらはスタイル計算自体をストリーミング化する将来のタスクで解消する。

use std::path::{Path, PathBuf};

use crate::fonts::{load_font_faces, load_missing_system_fonts, Font, FontCollection, SystemFonts};
use crate::html::StreamingParser;
use crate::layout::{paginate_document_streaming, PageSettings};
use crate::pdf::StreamingPdfWriter;
use crate::sink::Sink;
use crate::style::{compute_styles, extract_author_stylesheet, user_agent_stylesheet};

/// 一括処理かストリーミング処理かを選択する。
///
/// `Batch`はDOM全体が揃ってから処理する前提を明示する選択であり、
/// [0006](../docs/decisions/0006-css-non-locality-scope.md)が挙げた
/// 非局所性の制約を一切課さない。`Streaming`は同ADRの制約
/// (`<body>`より後の`<style>`タグをエラーにする)を適用する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Batch,
    Streaming,
}

/// `--font`相当の明示的なフォント指定。
pub struct FontSpec {
    pub path: PathBuf,
    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイルのフェイス番号。
    pub index: u32,
}

/// `Engine`の初期化オプション。
#[derive(Default)]
pub struct EngineOptions {
    pub mode: Mode,
    pub settings: PageSettings,
    /// `--font`相当の明示的なフォント指定(複数指定可)。
    pub fonts: Vec<FontSpec>,
    /// `@font-face`の`src: url(...)`を相対解決する基準ディレクトリ。
    /// 入力がファイルに対応しない場合(Rackボディ等)は`None`でよく、
    /// その場合はカレントディレクトリを基準にする。
    pub base_dir: Option<PathBuf>,
}

/// `Engine`が返すエラー。`Sink`からのエラー(`Io`)、コア自身が判定する
/// 構造エラー(`UnsupportedInStreamingMode`)、フォント読み込みエラー
/// (`Font`)を区別する。
#[derive(Debug)]
pub enum EngineError<E> {
    Io(E),
    UnsupportedInStreamingMode(&'static str),
    Font(String),
}

impl<E> From<E> for EngineError<E> {
    fn from(e: E) -> Self {
        Self::Io(e)
    }
}

impl<E: std::fmt::Display> std::fmt::Display for EngineError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::UnsupportedInStreamingMode(msg) => write!(f, "{msg}"),
            Self::Font(msg) => write!(f, "{msg}"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for EngineError<E> {}

/// HTMLチャンク投入からPDFバイト列書き出しまでを1つのAPIとして統合する
/// コアのエントリポイント。
pub struct Engine<S: Sink> {
    options: EngineOptions,
    parser: StreamingParser,
    sink: S,
}

impl<S: Sink> Engine<S> {
    pub fn new(options: EngineOptions, sink: S) -> Self {
        Self {
            options,
            parser: StreamingParser::new(),
            sink,
        }
    }

    /// HTMLバイト列のチャンクを1つ投入する。何度でも呼べる。
    ///
    /// `Mode::Streaming`では、投入後に`<body>`より後の`<style>`タグが
    /// 検出された場合エラーを返す(モジュールdoc参照)。`Mode::Batch`では
    /// このチェックを行わない。
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), EngineError<S::Error>> {
        self.parser.feed(chunk);
        if self.options.mode == Mode::Streaming && self.parser.has_late_style_tag() {
            return Err(EngineError::UnsupportedInStreamingMode(
                "<style> after <body> is not supported in streaming mode",
            ));
        }
        Ok(())
    }

    /// 残りの処理(DOM確定・フォント解決・スタイル計算・レイアウト・
    /// ページ分割・PDFエンコード)をすべて行い、`sink`へ書き出す。
    pub fn finish(self) -> Result<S::Output, EngineError<S::Error>> {
        let Self {
            options,
            parser,
            sink,
        } = self;
        let mut dom = parser.finish();

        let mut loaded_fonts = Vec::with_capacity(options.fonts.len());
        for spec in &options.fonts {
            let font = Font::load_indexed(&spec.path, spec.index)
                .map_err(|e| EngineError::Font(format!("フォントの読み込みに失敗しました: {e}")))?;
            loaded_fonts.push(font);
        }
        let mut fonts = FontCollection::new(loaded_fonts);

        let ua = user_agent_stylesheet();
        let author = extract_author_stylesheet(&dom);
        let styles = compute_styles(&dom, &ua, &author);

        // システムフォントのスキャン(メタデータのみ)は、`@font-face`の
        // `src: local(...)`解決でも使うため先に行っておく。
        let system_fonts = SystemFonts::scan();

        let base_dir = options
            .base_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."));
        for loaded in load_font_faces(&author.font_faces, base_dir, &system_fonts) {
            fonts.push_font_face(
                loaded.family,
                Some(loaded.weight),
                Some(loaded.style),
                loaded.font,
            );
        }
        load_missing_system_fonts(&mut fonts, &styles, &system_fonts);

        let mut writer =
            StreamingPdfWriter::new(&fonts, options.settings, sink).map_err(EngineError::Io)?;

        let mut write_error: Option<S::Error> = None;
        paginate_document_streaming(&mut dom, &styles, &fonts, &options.settings, &mut |page| {
            if write_error.is_some() {
                return;
            }
            if let Err(e) = writer.write_page(&page, &styles, &fonts) {
                write_error = Some(e);
            }
        });
        if let Some(e) = write_error {
            return Err(EngineError::Io(e));
        }

        writer.finish(&fonts).map_err(EngineError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{paginate_document, PageSettings};
    use crate::pdf::write_document;
    use crate::sink::MemorySink;

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn font_spec() -> FontSpec {
        FontSpec {
            path: PathBuf::from(DEJAVU_PATH),
            index: 0,
        }
    }

    fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn engine_produces_a_valid_pdf_from_a_single_feed() {
        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<p>Hello, world!</p>").unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"%%EOF") > 0);
        assert!(count_occurrences(&bytes, b"/MediaBox") > 0);
    }

    #[test]
    fn engine_produces_a_valid_pdf_from_multiple_feeds() {
        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<p>Hello").unwrap();
        engine.feed(b", ").unwrap();
        engine.feed(b"world!</p>").unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn engine_output_matches_the_batch_api_page_count() {
        let mut html_src = String::from("<style>.item { height: 100px; margin: 0; }</style><div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");

        let settings = PageSettings::default();

        // 既存の一括API経由。
        let dom = crate::html::parse(html_src.as_bytes());
        let ua = user_agent_stylesheet();
        let author = crate::style::extract_author_stylesheet(&dom);
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = FontCollection::new(vec![Font::load(DEJAVU_PATH).unwrap()]);
        let batched_pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(batched_pages.len() > 1, "expected multiple pages");
        let batched_bytes = write_document(
            &batched_pages,
            &styles,
            &fonts,
            &settings,
            MemorySink::new(),
        )
        .unwrap();

        // Engine経由。
        let options = EngineOptions {
            fonts: vec![font_spec()],
            settings,
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html_src.as_bytes()).unwrap();
        let engine_bytes = engine.finish().unwrap();

        assert_eq!(
            count_occurrences(&engine_bytes, b"/MediaBox"),
            count_occurrences(&batched_bytes, b"/MediaBox"),
            "Engine (streaming) and the batch API should produce the same page count"
        );
    }

    #[test]
    fn streaming_mode_rejects_a_style_tag_after_body_starts() {
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<body><p>x</p>").unwrap();

        match engine.feed(b"<style>p{color:red}</style>") {
            Err(EngineError::UnsupportedInStreamingMode(_)) => {}
            other => panic!("expected UnsupportedInStreamingMode, got {other:?}"),
        }
    }

    #[test]
    fn batch_mode_allows_a_style_tag_after_body_starts() {
        let options = EngineOptions {
            mode: Mode::Batch,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<body><p>x</p>").unwrap();
        engine
            .feed(b"<style>p{color:red}</style>")
            .expect("Mode::Batch should not reject a late <style> tag");
        let bytes = engine.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn engine_resolves_at_font_face_relative_to_base_dir() {
        // 既存のCLI E2Eテスト(cli.rs)と同じ@font-face+base_dir解決の
        // シナリオをEngine経由で検証する。
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-engine-test-{}-{}",
            std::process::id(),
            "font_face"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let font_dest = dir.join("embedded.ttf");
        std::fs::copy(DEJAVU_PATH, &font_dest).unwrap();

        let html = r#"<html><head><style>
            @font-face { font-family: "Embedded"; src: url("embedded.ttf"); }
            p { font-family: "Embedded"; }
        </style></head><body><p>hello</p></body></html>"#;

        let options = EngineOptions {
            base_dir: Some(dir.clone()),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"/FontFile2") > 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
