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
//! ## `Mode::Batch`と`Mode::Streaming`でパイプラインが異なる
//!
//! `Mode::Batch`は、`finish`が呼ばれた時点でDOM全体を一括して
//! (`compute_styles`/`build_box_tree`/`layout_document`/
//! `paginate_document_streaming`で)処理する、M1由来の一括APIの薄いラッパー。
//!
//! `Mode::Streaming`は、`<body>`直下のトップレベルブロック要素が確定する
//! たびに、そのサブツリーだけをスタイル計算・レイアウト・ページ分割・
//! PDF書き出し・DOM解放まで処理する「真のストリーミング処理」を行う
//! (詳細は[0010](../docs/decisions/0010-true-streaming-input.md)参照)。
//! `<html>`/`<body>`自身のスタイルは、最初のトップレベル要素が確定する
//! までに一度だけ計算し、以後の各トップレベル要素のスタイル計算の起点
//! (継承元)として使う。
//!
//! ### 既知の限界
//!
//! * フォントセット(`--font`明示指定+`@font-face`)は、最初のトップレベル
//!   要素を処理する前に確定させる。以後のトップレベル要素処理で新しい
//!   `font-family`が現れても、システムフォントの自動探索
//!   (`load_missing_system_fonts`)は行わない
//!   ([`crate::pdf::StreamingPdfWriter`]が`new`時点でフォント数を固定する
//!   ため、後から動的にフォントを追加できない)。`Mode::Streaming`を選ぶ
//!   場合は`--font`または`@font-face`で使用する全フォントを明示すること
//! * `<html>`/`<body>`自身に背景色・枠線がある場合、複数ページにまたがる
//!   装飾フラグメントの再現(`place_split`)をトップレベル要素単位の
//!   ストリーミングに対応させる必要があり複雑になるため非サポートとし、
//!   `Mode::Streaming`ではエラーを返す
//! * `nth-last-child`等の非局所セレクタ([0006]の分類3)は、各トップレベル
//!   要素のスタイル計算がそのサブツリー内で完結するため、`Mode::Streaming`
//!   では最初から構造的に評価できない(常に非マッチになる)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fonts::{load_font_faces, load_missing_system_fonts, Font, FontCollection, SystemFonts};
use crate::html::{NodeId, StreamingParser};
use crate::layout::{
    build_box_for_element, has_visible_decoration, layout_document_from,
    paginate_document_streaming, resolve_border, resolve_lpa_or_zero, resolve_padding,
    resolve_width_and_horizontal_margins, PageSettings, StreamingPaginator,
};
use crate::pdf::StreamingPdfWriter;
use crate::sink::Sink;
use crate::style::{
    compute_single_element_style, compute_styles, compute_styles_with_parent,
    extract_author_stylesheet, user_agent_stylesheet, ComputedStyle, Stylesheet,
};

/// 一括処理かストリーミング処理かを選択する。
///
/// `Batch`はDOM全体が揃ってから処理する前提を明示する選択であり、
/// [0006](../docs/decisions/0006-css-non-locality-scope.md)が挙げた
/// 非局所性の制約を一切課さない。`Streaming`は同ADRの制約
/// (`<body>`より後の`<style>`タグをエラーにする、非局所セレクタが常に
/// 非マッチになる)を適用し、モジュールdocに挙げた既知の限界も伴う。
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

/// `Mode::Streaming`でのトップレベル要素処理に必要な、`<head>`閉じ時点
/// (`<body>`検出時点)で一度だけ確定する状態。
struct StreamingState<S: Sink> {
    ua: Stylesheet,
    author: Stylesheet,
    fonts: FontCollection,
    /// 処理済みの全トップレベル要素のスタイルを蓄積する、永続的なマップ。
    /// 1ページに複数のトップレベル要素のボックスが混在しうるため、
    /// `StreamingPdfWriter::write_page`はこの全体を必要とする。
    styles: HashMap<NodeId, ComputedStyle>,
    root_font_size: f32,
    /// `<body>`要素自身の計算スタイル。各トップレベル要素のスタイル計算の
    /// 親スタイルとして使う。
    body_style: ComputedStyle,
    /// `<body>`の`padding`/`border`/`margin`を反映した、トップレベル要素の
    /// containing width。
    content_width: f32,
    /// `<body>`の`margin-left`+`border-left`+`padding-left`。
    start_x: f32,
    /// 次に配置するトップレベル要素の開始Y座標(前の要素までの累積高さ)。
    cursor_y: f32,
    paginator: StreamingPaginator,
    writer: StreamingPdfWriter<S>,
}

/// HTMLチャンク投入からPDFバイト列書き出しまでを1つのAPIとして統合する
/// コアのエントリポイント。
pub struct Engine<S: Sink> {
    options: EngineOptions,
    parser: StreamingParser,
    /// `Mode::Batch`では`finish`まで保持し続ける。`Mode::Streaming`では
    /// 最初のトップレベル要素処理の直前に`StreamingState::writer`へ
    /// 移動するため`None`になる。
    sink: Option<S>,
    streaming: Option<StreamingState<S>>,
}

impl<S: Sink> Engine<S> {
    pub fn new(options: EngineOptions, sink: S) -> Self {
        Self {
            options,
            parser: StreamingParser::new(),
            sink: Some(sink),
            streaming: None,
        }
    }

    /// HTMLバイト列のチャンクを1つ投入する。何度でも呼べる。
    ///
    /// `Mode::Streaming`では、投入後に`<body>`より後の`<style>`タグが
    /// 検出された場合エラーを返す(モジュールdoc参照)。`Mode::Batch`では
    /// このチェックを行わず、DOMを蓄積するのみで実際の処理は`finish`まで
    /// 行わない。`Mode::Streaming`では、確定した`<body>`直下のトップレベル
    /// 要素をこの中で処理する。
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), EngineError<S::Error>> {
        self.parser.feed(chunk);
        if self.options.mode == Mode::Streaming && self.parser.has_late_style_tag() {
            return Err(EngineError::UnsupportedInStreamingMode(
                "<style> after <body> is not supported in streaming mode",
            ));
        }

        if self.options.mode != Mode::Streaming {
            return Ok(());
        }

        self.ensure_streaming_state_initialized()?;
        if self.streaming.is_some() {
            let completed = self.parser.take_completed_top_level_children();
            for node in completed {
                self.process_top_level_element(node)?;
            }
        }
        Ok(())
    }

    /// `<body>`が検出されていて、まだ`StreamingState`を作っていなければ
    /// 作る。`sink`をここで`StreamingState::writer`へ移動する(以後
    /// `self.sink`は`None`になる)。
    fn ensure_streaming_state_initialized(&mut self) -> Result<(), EngineError<S::Error>> {
        if self.streaming.is_some() {
            return Ok(());
        }
        let Some(body) = self.parser.body_node() else {
            return Ok(());
        };
        let sink = self
            .sink
            .take()
            .expect("sinkはstreaming state初期化時に一度だけ取り出される");
        let state = self.init_streaming_state(body, sink)?;
        self.streaming = Some(state);
        Ok(())
    }

    /// `<head>`閉じ時点(`<body>`検出時点)で一度だけ行う初期化:
    /// フォント解決・`<html>`/`<body>`のスタイル計算・`<body>`の装飾
    /// チェック・`StreamingPdfWriter`の構築。
    fn init_streaming_state(
        &self,
        body: NodeId,
        sink: S,
    ) -> Result<StreamingState<S>, EngineError<S::Error>> {
        let ua = user_agent_stylesheet();
        let author = {
            let dom = self.parser.dom();
            extract_author_stylesheet(&dom)
        };

        let mut loaded_fonts = Vec::with_capacity(self.options.fonts.len());
        for spec in &self.options.fonts {
            let font = Font::load_indexed(&spec.path, spec.index)
                .map_err(|e| EngineError::Font(format!("フォントの読み込みに失敗しました: {e}")))?;
            loaded_fonts.push(font);
        }
        let mut fonts = FontCollection::new(loaded_fonts);

        let system_fonts = SystemFonts::scan();
        let base_dir = self
            .options
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
        // `load_missing_system_fonts`は文書全体のスタイルを必要とするが、
        // 真のストリーミング処理では文書全体のスタイルを一度に持たない
        // ため、ここでは呼ばない(モジュールdocの既知の限界を参照)。

        let (html_style, body_style, root_font_size) = {
            let dom = self.parser.dom();
            let html_id = dom
                .parent(body)
                .expect("<body>には親要素(<html>)があるはず");
            let default_root_font_size = ComputedStyle::default().font_size.0;
            let html_style = compute_single_element_style(
                &dom,
                html_id,
                None,
                default_root_font_size,
                &ua,
                &author,
            );
            let root_font_size = html_style.font_size.0;
            let body_style = compute_single_element_style(
                &dom,
                body,
                Some(&html_style),
                root_font_size,
                &ua,
                &author,
            );
            (html_style, body_style, root_font_size)
        };
        let _ = html_style;

        let body_border = resolve_border(&body_style);
        if has_visible_decoration(&body_style, &body_border) {
            return Err(EngineError::UnsupportedInStreamingMode(
                "<body> with a visible background/border is not supported in streaming mode",
            ));
        }

        let page_width = self.options.settings.content_width();
        let body_padding = resolve_padding(&body_style, page_width);
        let (body_content_width, body_margin_left, _) = resolve_width_and_horizontal_margins(
            &body_style,
            page_width,
            body_padding.left + body_padding.right,
            body_border.left + body_border.right,
        );
        let start_x = body_margin_left + body_border.left + body_padding.left;
        let start_y = resolve_lpa_or_zero(body_style.margin_top, page_width)
            + body_border.top
            + body_padding.top;

        let writer = StreamingPdfWriter::new(&fonts, self.options.settings, sink)
            .map_err(EngineError::Io)?;

        Ok(StreamingState {
            ua,
            author,
            fonts,
            styles: HashMap::new(),
            root_font_size,
            body_style,
            content_width: body_content_width,
            start_x,
            cursor_y: start_y,
            paginator: StreamingPaginator::new(self.options.settings.content_height()),
            writer,
        })
    }

    /// 確定した1つのトップレベル要素(`<body>`直下の子)を、スタイル計算・
    /// レイアウト・ページ分割・PDF書き出し・DOM解放まで処理する。
    fn process_top_level_element(&mut self, node: NodeId) -> Result<(), EngineError<S::Error>> {
        let Engine {
            parser, streaming, ..
        } = self;
        let state = streaming
            .as_mut()
            .expect("process_top_level_elementはstreaming state初期化後にのみ呼ばれる");

        let (sub_styles, item_box) = {
            let dom = parser.dom();
            let sub_styles = compute_styles_with_parent(
                &dom,
                node,
                &state.body_style,
                state.root_font_size,
                &state.ua,
                &state.author,
            );
            let item_box = build_box_for_element(&dom, &sub_styles, node);
            (sub_styles, item_box)
        };
        state.styles.extend(sub_styles);

        let Some(item_box) = item_box else {
            // `display: none`などでボックスを生成しない要素。
            parser.dom_mut().release_subtree(node);
            return Ok(());
        };

        let laid_out = layout_document_from(
            &item_box,
            &state.styles,
            &state.fonts,
            state.content_width,
            state.start_x,
            state.cursor_y,
        );
        state.cursor_y += laid_out.layout.margin_box_height();

        // レイアウトはすでに完了しており、これ以降このDOMサブツリー
        // (テキスト内容・属性等)が再度読まれることはないため、ページの
        // flushを待たずに即座に解放してよい(`ComputedStyle`は`state.styles`
        // に別途保持済み)。
        parser.dom_mut().release_subtree(node);

        let pages = state.paginator.push_item(&laid_out);
        for page in pages {
            state
                .writer
                .write_page(&page, &state.styles, &state.fonts)
                .map_err(EngineError::Io)?;
        }

        Ok(())
    }

    /// 残りの処理をすべて行い、`sink`へ書き出す。
    ///
    /// `Mode::Batch`ではDOM確定後に一括処理する。`Mode::Streaming`では、
    /// まだ処理していない(保留中だった最後の要素を含む)トップレベル要素を
    /// すべて処理してから、`StreamingPdfWriter::finish`でフォント埋め込み・
    /// xref/trailerを書き出す。
    pub fn finish(mut self) -> Result<S::Output, EngineError<S::Error>> {
        if self.options.mode != Mode::Streaming {
            return self.finish_batch();
        }

        self.ensure_streaming_state_initialized()?;
        let remaining = self.parser.take_all_remaining_top_level_children();
        for node in remaining {
            self.process_top_level_element(node)?;
        }

        match self.streaming {
            Some(state) => {
                let StreamingState {
                    styles,
                    fonts,
                    mut writer,
                    paginator,
                    ..
                } = state;
                for page in paginator.finish() {
                    writer
                        .write_page(&page, &styles, &fonts)
                        .map_err(EngineError::Io)?;
                }
                writer.finish(&fonts).map_err(EngineError::Io)
            }
            None => {
                // <body>が一度も現れなかった(空文書・不正な入力等)。
                // 空のsink(0ページのPDFにはならないが、書き込みなしで
                // finishする)扱いにする。
                let sink = self
                    .sink
                    .take()
                    .expect("streaming未初期化ならsinkはまだ保持しているはず");
                sink.finish().map_err(EngineError::Io)
            }
        }
    }

    fn finish_batch(self) -> Result<S::Output, EngineError<S::Error>> {
        let Self {
            options,
            parser,
            sink,
            ..
        } = self;
        let mut dom = parser.finish();
        let sink = sink.expect("Mode::Batchではsinkがfinishまでそのまま保持される");

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
    use crate::layout::paginate_document;
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

        // Engine経由(Mode::Batch)。
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
            "Engine (batch mode) and the batch API should produce the same page count"
        );
    }

    #[test]
    fn streaming_mode_produces_the_same_page_count_as_batch_mode() {
        let mut html_src = String::from("<style>.item { height: 100px; margin: 0; }</style><div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");

        let settings = PageSettings::default();

        let batch_options = EngineOptions {
            mode: Mode::Batch,
            fonts: vec![font_spec()],
            settings,
            ..EngineOptions::default()
        };
        let mut batch_engine = Engine::new(batch_options, MemorySink::new());
        batch_engine.feed(html_src.as_bytes()).unwrap();
        let batch_bytes = batch_engine.finish().unwrap();
        let batch_pages = count_occurrences(&batch_bytes, b"/MediaBox");
        assert!(batch_pages > 1, "expected multiple pages");

        let streaming_options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            settings,
            ..EngineOptions::default()
        };
        let mut streaming_engine = Engine::new(streaming_options, MemorySink::new());
        streaming_engine.feed(html_src.as_bytes()).unwrap();
        let streaming_bytes = streaming_engine.finish().unwrap();

        assert_eq!(
            count_occurrences(&streaming_bytes, b"/MediaBox"),
            batch_pages,
            "Mode::Streaming should produce the same page count as Mode::Batch"
        );
    }

    #[test]
    fn streaming_mode_works_when_fed_one_byte_at_a_time() {
        let mut html_src =
            String::from("<style>.item { height: 100px; margin: 0; }</style><body><div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div></body>");

        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            settings: PageSettings::default(),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        for byte in html_src.as_bytes() {
            engine.feed(std::slice::from_ref(byte)).unwrap();
        }
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"/MediaBox") > 1);
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
    fn streaming_mode_rejects_a_decorated_body() {
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        match engine.feed(
            b"<html><head><style>body { background-color: red; }</style></head><body><p>x</p>",
        ) {
            Err(EngineError::UnsupportedInStreamingMode(_)) => {}
            other => panic!("expected UnsupportedInStreamingMode, got {other:?}"),
        }
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
