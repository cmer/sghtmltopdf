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
use std::rc::Rc;

use crate::fonts::{load_font_faces, load_missing_system_fonts, Font, FontCollection, SystemFonts};
use crate::html::{collect_anchor_targets, find_base_href, Dom, NodeId, StreamingParser};
use crate::img::{DocumentImageCache, ImageFetcher};
use crate::layout::{
    build_box_for_element, collect_completed_subtree_roots, has_visible_decoration,
    layout_document_from, paginate_document, paginate_document_streaming,
    resolve_background_images, resolve_border, resolve_images, resolve_lpa_or_zero,
    resolve_padding, resolve_width_and_horizontal_margins, PageSettings, StreamingPaginator,
};
use crate::pdf::{
    anchor_destination_name, ImageAssetCache, LinkSettings, PreparedImage, StreamingPdfWriter,
};
use crate::sink::Sink;
use crate::style::{
    compute_single_element_style, compute_styles, compute_styles_with_parent,
    extract_author_stylesheet, resolve_page_rules, rules_use_page_count, user_agent_stylesheet,
    ComputedStyle, LengthPercentageOrAuto, PageRule, Stylesheet,
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
    /// その場合はカレントディレクトリを基準にする。`<img src>`のローカル
    /// 相対パス解決にも同じ基準ディレクトリを使う。
    pub base_dir: Option<PathBuf>,
    /// `<img src>`・`<link rel=stylesheet href>`のhttp(s)絶対URLフェッチを
    /// 許可するか。既定`false`([0013](../docs/decisions/0013-image-fetch-security.md)
    /// の「既定無効・明示オプトイン」方針。[0015](../docs/decisions/0015-external-stylesheet-fetch-design.md)
    /// 決定2により、画像・外部スタイルシート双方をこの1つのフラグで
    /// 統括する)。ローカル相対パス・`data:`URIはこの値に関わらず常に許可する。
    pub allow_remote_assets: bool,
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
    /// `background-image`を持つ要素の、デコード済み画像を`NodeId`キーで
    /// 引けるようにする側マップ。`styles`と同じく処理済みトップレベル要素
    /// ぶんを蓄積する([0017](../docs/decisions/0017-background-image-design.md)決定2)。
    background_images: HashMap<NodeId, Rc<PreparedImage>>,
    root_font_size: f32,
    /// CSSカウンタの状態([0024](../docs/decisions/0024-generated-content-design.md)
    /// 決定2)。ドキュメント順に依存するため、トップレベル要素をまたいで
    /// 永続させる必要があり`root_font_size`と同じ位置づけで持つ。
    counters: HashMap<String, Vec<i32>>,
    /// `quotes`のネスト深度(決定3、木構造とは無関係な単一のカウンタ)。
    quote_depth: i32,
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
    /// `<img>`のフェッチ・デコード結果を文書内でメモ化するキャッシュ。
    image_cache: ImageAssetCache,
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
        if self.options.mode == Mode::Streaming && self.parser.has_late_css_source() {
            return Err(EngineError::UnsupportedInStreamingMode(
                "<style>/<link rel=stylesheet> after <body> is not supported in streaming mode",
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
        let base_dir = self
            .options
            .base_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."));
        // 外部スタイルシート(`<link>`)取得用のフェッチャー/キャッシュ。
        // 画像用の`ImageAssetCache`(下の`image_cache`)とは別インスタンスを
        // 持つ([0015](../docs/decisions/0015-external-stylesheet-fetch-design.md)
        // 決定3)。
        // `<base href>`は`<head>`に現れるため、この時点(最初のトップレベル
        // 要素が確定した時点)で既にパース済み([0040](
        // ../docs/decisions/0040-base-href-design.md)決定3)。
        let base_href = find_base_href(&self.parser.dom());
        let css_fetcher =
            ImageFetcher::new(base_dir.to_path_buf(), self.options.allow_remote_assets)
                .with_base_href(base_href.clone());
        let css_cache = DocumentImageCache::new();
        let author = {
            let dom = self.parser.dom();
            extract_author_stylesheet(&dom, &css_fetcher, &css_cache)
        };
        let page_settings =
            apply_page_rule_settings_override(self.options.settings, &author.page_rules);
        // `counter(pages)`は文書全体のページ分割完了まで値が定まらないため、
        // 真のストリーミング処理とは原理的に相容れない([0028](
        // ../docs/decisions/0028-paged-media-design.md)決定6、ユーザー確認済み)。
        if rules_use_page_count(&author.page_rules) {
            return Err(EngineError::UnsupportedInStreamingMode(
                "counter(pages) in @page margin boxes is not supported in streaming mode",
            ));
        }

        let mut loaded_fonts = Vec::with_capacity(self.options.fonts.len());
        for spec in &self.options.fonts {
            let font = Font::load_indexed(&spec.path, spec.index)
                .map_err(|e| EngineError::Font(format!("フォントの読み込みに失敗しました: {e}")))?;
            loaded_fonts.push(font);
        }
        let mut fonts = FontCollection::new(loaded_fonts);

        let system_fonts = SystemFonts::scan();
        for loaded in load_font_faces(&author.font_faces, base_dir, &system_fonts) {
            fonts.push_font_face(
                loaded.family,
                Some(loaded.weight),
                Some(loaded.style),
                loaded.unicode_range,
                loaded.font,
            );
        }
        // `load_missing_system_fonts`は文書全体のスタイルを必要とするが、
        // 真のストリーミング処理では文書全体のスタイルを一度に持たない
        // ため、ここでは呼ばない(モジュールdocの既知の限界を参照)。

        // CSSカウンタ・quote深度はドキュメント順に依存する状態([0024]決定2・3)
        // なので、<html>から<body>直下の各トップレベル要素まで一貫して
        // 同じ状態を引き継ぐ(以後`StreamingState`が永続させる)。
        let mut counters = HashMap::new();
        let mut quote_depth = 0;
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
                &mut counters,
                &mut quote_depth,
            );
            let root_font_size = html_style.font_size.0;
            let body_style = compute_single_element_style(
                &dom,
                body,
                Some(&html_style),
                root_font_size,
                &ua,
                &author,
                &mut counters,
                &mut quote_depth,
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

        // `<a href="#id">`の宛先候補([0042](
        // ../docs/decisions/0042-link-annotations-design.md)決定4)。
        // `Mode::Streaming`ではこの時点(最初のトップレベル要素が確定した
        // 時点)までにパースできた範囲しか見えないが、宛先は「そのページを
        // 書き出す時に見つかったボックス」から記録されるため、後から
        // パースされる要素も対象になる(ここで集めるのは`id`の一覧ではなく、
        // 「どのノードがどの名前か」の対応表であるため)。
        let anchor_names: HashMap<NodeId, String> = collect_anchor_targets(&self.parser.dom())
            .into_iter()
            .map(|(node, id)| (node, anchor_destination_name(&id)))
            .collect();

        let page_width = page_settings.content_width();
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

        let writer = StreamingPdfWriter::new(
            &fonts,
            page_settings,
            sink,
            author.page_rules.clone(),
            LinkSettings {
                anchor_names: anchor_names.clone(),
                base_href: base_href.clone(),
            },
        )
        .map_err(EngineError::Io)?;
        let image_cache = ImageAssetCache::with_base_href(
            base_dir.to_path_buf(),
            self.options.allow_remote_assets,
            base_href,
        );

        Ok(StreamingState {
            ua,
            author,
            fonts,
            styles: HashMap::new(),
            background_images: HashMap::new(),
            root_font_size,
            counters,
            quote_depth,
            body_style,
            content_width: body_content_width,
            start_x,
            cursor_y: start_y,
            paginator: StreamingPaginator::new(page_settings.content_height()),
            writer,
            image_cache,
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
                &mut state.counters,
                &mut state.quote_depth,
            );
            let mut item_box = build_box_for_element(&dom, &sub_styles, node);
            if let Some(item_box) = &mut item_box {
                resolve_images(item_box, &dom, &state.image_cache);
            }
            (sub_styles, item_box)
        };
        state
            .background_images
            .extend(resolve_background_images(&sub_styles, &state.image_cache));
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

        // このトップレベル要素自体が装飾(背景・枠線・background-image、
        // [0017]決定2により`has_visible_decoration`はbackground-imageも
        // 見る)を持たない場合、`place_split`は装飾フラグメントを生成しない
        // ため、このノード自体が`page.boxes`に現れることはない。つまり
        // `node`自身の`ComputedStyle`/背景画像はこの後`write_page`から
        // 一切参照されないため、ここで即座に削除してよい(装飾を持つ場合は、
        // 装飾フラグメントが実際に配置されたページのflush時に、下の
        // `collect_completed_subtree_roots`経由で削除される)。
        if !laid_out.has_visible_decoration {
            state.styles.remove(&node);
            state.background_images.remove(&node);
        }

        let pages = state.paginator.push_item(&laid_out);
        for page in &pages {
            state
                .writer
                // `Mode::Streaming`は総ページ数を原理的に知りえないため常に`None`
                // (`counter(pages)`使用時は`init_streaming_state`で事前に
                // エラーを返している)。
                .write_page(
                    page,
                    &state.styles,
                    &state.background_images,
                    &state.fonts,
                    None,
                )
                .map_err(EngineError::Io)?;
        }

        // 各ページに実際に配置され、これ以上分割されない
        // (`FragmentPosition::Whole`/`Last`)子孫ノードの`ComputedStyle`/
        // 背景画像を解放する。DOM自体は上ですでにタブストーン化済みだが、
        // 木構造のリンクは保持されているため`Dom::children`で辿れる。
        let dom = parser.dom();
        for page in &pages {
            for root in collect_completed_subtree_roots(page) {
                remove_subtree_styles(&dom, root, &mut state.styles, &mut state.background_images);
            }
        }
        drop(dom);

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
                    background_images,
                    fonts,
                    mut writer,
                    paginator,
                    ..
                } = state;
                for page in paginator.finish() {
                    writer
                        .write_page(&page, &styles, &background_images, &fonts, None)
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
        let base_dir = options
            .base_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."));
        let base_href = find_base_href(&dom);
        let css_fetcher = ImageFetcher::new(base_dir.to_path_buf(), options.allow_remote_assets)
            .with_base_href(base_href.clone());
        let css_cache = DocumentImageCache::new();
        let author = extract_author_stylesheet(&dom, &css_fetcher, &css_cache);
        let styles = compute_styles(&dom, &ua, &author);
        // `<a href="#id">`の宛先候補([0042]決定4)。
        let anchor_names: HashMap<NodeId, String> = collect_anchor_targets(&dom)
            .into_iter()
            .map(|(node, id)| (node, anchor_destination_name(&id)))
            .collect();
        let page_settings = apply_page_rule_settings_override(options.settings, &author.page_rules);

        let system_fonts = SystemFonts::scan();
        for loaded in load_font_faces(&author.font_faces, base_dir, &system_fonts) {
            fonts.push_font_face(
                loaded.family,
                Some(loaded.weight),
                Some(loaded.style),
                loaded.unicode_range,
                loaded.font,
            );
        }
        load_missing_system_fonts(&mut fonts, &styles, &system_fonts);

        // `counter(pages)`が使われている場合のみ、総ページ数を確定させる
        // ための事前カウント用パスを走らせる(レイアウト・ページ分割の
        // 計算コストが余分にかかるが、この機能を使わない文書には一切
        // 影響しない、[0028](../docs/decisions/0028-paged-media-design.md)
        // 決定6)。`Mode::Batch`はこの時点で`dom`全体が確定済みなので
        // 事前カウントが可能(`Mode::Streaming`では原理的に不可能、
        // `init_streaming_state`で別途エラーにしている)。
        let total_pages = if rules_use_page_count(&author.page_rules) {
            Some(paginate_document(&dom, &styles, &fonts, &page_settings).len())
        } else {
            None
        };

        let mut writer = StreamingPdfWriter::new(
            &fonts,
            page_settings,
            sink,
            author.page_rules.clone(),
            LinkSettings {
                anchor_names: anchor_names.clone(),
                base_href: base_href.clone(),
            },
        )
        .map_err(EngineError::Io)?;
        let image_cache = ImageAssetCache::with_base_href(
            base_dir.to_path_buf(),
            options.allow_remote_assets,
            base_href,
        );
        // `background-image`はレイアウトのサイズ計算に影響しない描画専用の
        // 情報なので、`resolve_images`(box tree構築)とは独立に、文書全体の
        // `styles`から一度だけ構築できる([0017]決定2)。
        let background_images = resolve_background_images(&styles, &image_cache);

        let mut write_error: Option<S::Error> = None;
        paginate_document_streaming(
            &mut dom,
            &styles,
            &fonts,
            &page_settings,
            &image_cache,
            &mut |page| {
                if write_error.is_some() {
                    return;
                }
                if let Err(e) =
                    writer.write_page(&page, &styles, &background_images, &fonts, total_pages)
                {
                    write_error = Some(e);
                }
            },
        );
        if let Some(e) = write_error {
            return Err(EngineError::Io(e));
        }

        writer.finish(&fonts).map_err(EngineError::Io)
    }
}

/// `@page`ルールの`size`/`margin`宣言(無条件`@page{}`ルールのみ、
/// [0028](../docs/decisions/0028-paged-media-design.md)決定4改訂)を
/// `base`(CLIオプション/既定値)へ適用した`PageSettings`を返す。
/// `:first`/`:left`/`:right`はmargin box(ヘッダー/フッター内容)の出し分け
/// にのみ使うため、ここでは無条件ルールだけを見ればよい
/// (`is_first`/`is_left`はどちらの値でも`resolve_page_rules`が返す
/// `size_px`/`margin_*`には影響しない)。
fn apply_page_rule_settings_override(base: PageSettings, page_rules: &[PageRule]) -> PageSettings {
    let resolved = resolve_page_rules(page_rules, false, false);
    let mut settings = base;
    if let Some((width, height)) = resolved.size_px {
        settings.size.width = width;
        settings.size.height = height;
    }
    let resolve_edge = |value: Option<LengthPercentageOrAuto>, base: f32, basis: f32| match value {
        None | Some(LengthPercentageOrAuto::Auto) => base,
        Some(LengthPercentageOrAuto::LengthPercentage(lp)) => match lp {
            crate::style::LengthPercentage::Length(px) => px,
            crate::style::LengthPercentage::Percentage(p) => basis * p,
        },
    };
    settings.margin.top = resolve_edge(
        resolved.margin_top,
        settings.margin.top,
        settings.size.height,
    );
    settings.margin.bottom = resolve_edge(
        resolved.margin_bottom,
        settings.margin.bottom,
        settings.size.height,
    );
    settings.margin.left = resolve_edge(
        resolved.margin_left,
        settings.margin.left,
        settings.size.width,
    );
    settings.margin.right = resolve_edge(
        resolved.margin_right,
        settings.margin.right,
        settings.size.width,
    );
    settings
}

/// `root`以下のサブツリーに属するノードの`ComputedStyle`を`styles`から
/// 取り除く。`dom`は`root`以下がすでに[`Dom::release_subtree`]で解放済み
/// (タブストーン化済み)でもよい(木構造のリンク自体は保持されるため)。
fn remove_subtree_styles(
    dom: &Dom,
    root: NodeId,
    styles: &mut HashMap<NodeId, ComputedStyle>,
    background_images: &mut HashMap<NodeId, Rc<PreparedImage>>,
) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        stack.extend(dom.children(id));
        styles.remove(&id);
        background_images.remove(&id);
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

    /// PDFバイト列中の全`stream`〜`endstream`区間を展開して連結したものを
    /// 返す。各ストリームの`/Length N`をパースし、`stream\n`直後から正確に
    /// `N`バイトを切り出す(`core/src/pdf/document.rs`の同名ヘルパーは
    /// `\nendstream`という文字列を素朴に探すだけで、フォント埋め込み
    /// バイナリ中に偶然そのバイト列が出現すると誤って区切ってしまい
    /// 後続のストリームを取りこぼす。それを踏んで`sanity check: batched
    /// output should draw strokes`が誤って失敗することを実際に確認した
    /// ため、ここでは`/Length`を使う正確な実装にしている)。
    fn decompressed_stream_bytes(pdf_bytes: &[u8]) -> Vec<u8> {
        fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack.windows(needle.len()).position(|w| w == needle)
        }

        let mut out = Vec::new();
        let mut i = 0;
        // 末尾の空白で`/Length1`(フォントの元サイズ)と区別する。
        while let Some(pos) = find_subslice(&pdf_bytes[i..], b"/Length ") {
            let len_start = i + pos + b"/Length ".len();
            let mut len_end = len_start;
            while len_end < pdf_bytes.len() && pdf_bytes[len_end].is_ascii_digit() {
                len_end += 1;
            }
            let Some(length) = std::str::from_utf8(&pdf_bytes[len_start..len_end])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
            else {
                i = len_end.max(i + pos + 1);
                continue;
            };
            let Some(stream_rel) = find_subslice(&pdf_bytes[len_end..], b"stream\n") else {
                break;
            };
            let data_start = len_end + stream_rel + b"stream\n".len();
            let data_end = data_start + length;
            if data_end > pdf_bytes.len() {
                i = len_end;
                continue;
            }
            let raw = &pdf_bytes[data_start..data_end];

            let mut decoder = flate2::read::ZlibDecoder::new(raw);
            let mut decompressed = Vec::new();
            if std::io::Read::read_to_end(&mut decoder, &mut decompressed).is_ok() {
                out.extend_from_slice(&decompressed);
            } else {
                out.extend_from_slice(raw);
            }
            out.push(b'\n');

            i = data_end;
        }
        out
    }

    #[test]
    fn streaming_mode_releases_computed_styles_for_flushed_pages() {
        // 装飾のない200個の<p>。全要素分の`ComputedStyle`を`finish`まで
        // 保持し続けるなら、200要素分(400エントリ超)が`styles`に残るはず。
        // ページがflushされるたびに解放されていれば、直近の未flushページ
        // 分程度(数十エントリ)に収まる。
        let mut html_src = String::from("<style>.item { height: 100px; margin: 0; }</style><body>");
        for i in 0..200 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</body>");

        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            settings: PageSettings::default(),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html_src.as_bytes()).unwrap();

        let styles_len = engine
            .streaming
            .as_ref()
            .expect("<body> should have been detected by now")
            .styles
            .len();
        assert!(
            styles_len < 50,
            "expected the styles map to stay small while streaming (pages should \
             release their entries once flushed), but it grew to {styles_len} entries"
        );

        let bytes = engine.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
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
    fn streaming_mode_matches_batch_mode_for_a_decorated_wrapper_spanning_pages() {
        // 単一のトップレベル要素(背景色・枠線を持つwrapper)が複数ページに
        // またがるケース。`process_top_level_element`は1回しか呼ばれない
        // ため、`push_item`の1回の呼び出し内で複数ページがflushされる。
        // `styles`解放ロジック(`collect_completed_subtree_roots`)が、
        // wrapper自身の`ComputedStyle`をまだ必要な間に誤って消していないか
        // どうかは、`render_box`が`styles.get`の失敗をサイレントに
        // `ComputedStyle::default()`へフォールバックしてしまう
        // (`core/src/pdf/document.rs`)ため、ページ数の一致だけでは検出
        // できない可能性がある。出力バイト列そのものを一括APIと比較する。
        let mut html_src = String::from(r#"<div class="wrapper">"#);
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");

        let author_css = ".wrapper { border: 2px solid black; padding: 5px; margin: 0; } \
             .item { height: 100px; margin: 0; }";
        let settings = PageSettings::default();

        let dom = crate::html::parse(html_src.as_bytes());
        let ua = user_agent_stylesheet();
        let author = crate::style::parse_stylesheet(author_css);
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = FontCollection::new(vec![Font::load(DEJAVU_PATH).unwrap()]);
        let batched_pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(batched_pages.len() > 1, "expected multiple pages");
        let batched_bytes = write_document(
            &batched_pages,
            &styles,
            &HashMap::new(),
            &fonts,
            &settings,
            MemorySink::new(),
        )
        .unwrap();

        let html_with_style = format!("<style>{author_css}</style>{html_src}");
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            settings,
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html_with_style.as_bytes()).unwrap();
        let streamed_bytes = engine.finish().unwrap();

        assert_eq!(
            count_occurrences(&streamed_bytes, b"/MediaBox"),
            count_occurrences(&batched_bytes, b"/MediaBox"),
        );
        // 描画コンテンツ(枠線描画で使われる`closepath`+`fill`の出現数)も
        // 一致するはず。`styles`から早すぎるタイミングでwrapperの
        // `ComputedStyle`が失われていれば、装飾(枠線)の描画コマンドが欠落
        // しこの数が変わる。コンテンツストリームは`/FlateDecode`で圧縮
        // されているため、圧縮後の`bytes`を直接文字列検索しても意味が
        // なく、展開してから比較する必要がある(`solid_border_fills_a_
        // mitered_quad_per_side`が示す通り、単色borderは`stroke`ではなく
        // 辺ごとの塗りつぶしパスとして描画される実装のため`h\nf\n`を数える)。
        let streamed_stream = decompressed_stream_bytes(&streamed_bytes);
        let batched_stream = decompressed_stream_bytes(&batched_bytes);
        let streamed_fills = count_occurrences(&streamed_stream, b"h\nf\n");
        let batched_fills = count_occurrences(&batched_stream, b"h\nf\n");
        assert!(
            batched_fills > 0,
            "sanity check: batched output should draw border fill paths"
        );
        assert_eq!(
            streamed_fills, batched_fills,
            "border fill path count should match (border rendering should be identical)"
        );
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
        let css_fetcher = ImageFetcher::new(std::path::PathBuf::from("."), false);
        let css_cache = DocumentImageCache::new();
        let author = crate::style::extract_author_stylesheet(&dom, &css_fetcher, &css_cache);
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = FontCollection::new(vec![Font::load(DEJAVU_PATH).unwrap()]);
        let batched_pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(batched_pages.len() > 1, "expected multiple pages");
        let batched_bytes = write_document(
            &batched_pages,
            &styles,
            &HashMap::new(),
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
    fn apply_page_rule_settings_override_uses_only_unconditional_rules() {
        let base = PageSettings::default();
        let sheet = crate::style::parse_stylesheet(
            "@page { size: 300px 400px; margin: 20px; } \
             @page :first { size: 999px 999px; margin: 999px; }",
        );
        let overridden = apply_page_rule_settings_override(base, &sheet.page_rules);
        assert_eq!(overridden.size.width, 300.0);
        assert_eq!(overridden.size.height, 400.0);
        assert_eq!(overridden.margin.top, 20.0);
        assert_eq!(overridden.margin.left, 20.0);
    }

    #[test]
    fn apply_page_rule_settings_override_leaves_settings_unchanged_without_at_page() {
        let base = PageSettings::default();
        let overridden = apply_page_rule_settings_override(base, &[]);
        assert_eq!(overridden, base);
    }

    #[test]
    fn at_page_size_overrides_the_pdf_media_box_in_batch_mode() {
        let options = EngineOptions {
            mode: Mode::Batch,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine
            .feed(b"<html><head><style>@page { size: 300px 400px; }</style></head><body><p>x</p></body></html>")
            .unwrap();
        let bytes = engine.finish().unwrap();
        assert!(
            count_occurrences(&bytes, b"/MediaBox [0 0 300 400]") > 0,
            "@page size should override the PDF MediaBox"
        );
    }

    #[test]
    fn at_page_size_overrides_the_pdf_media_box_in_streaming_mode() {
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine
            .feed(b"<html><head><style>@page { size: 300px 400px; }</style></head><body><p>x</p></body></html>")
            .unwrap();
        let bytes = engine.finish().unwrap();
        assert!(
            count_occurrences(&bytes, b"/MediaBox [0 0 300 400]") > 0,
            "@page size should override the PDF MediaBox in streaming mode too"
        );
    }

    #[test]
    fn margin_box_content_glyphs_are_embedded_in_the_font_subset_in_batch_mode() {
        // margin boxのcontentは通常のBoxContent::Inline経路(collect_usage)を
        // 通らない独立した経路(collect_margin_box_usage)なので、専用の収集漏れが
        // 起きていないかを回帰確認する(M8のマーカーグリフ収集漏れと同種のバグ
        // クラス)。本文には登場しない数字を`@bottom-right`のページ番号として
        // 表示させ、そのグリフが実際にToUnicode CMapへ埋め込まれることを確認する。
        let options = EngineOptions {
            mode: Mode::Batch,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine
            .feed(
                b"<html><head><style>\
                    @page { @bottom-right { content: counter(page); } }\
                  </style></head><body><p>no digits here</p></body></html>",
            )
            .unwrap();
        let bytes = engine.finish().unwrap();
        let decompressed = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&decompressed, b"<0031>") > 0,
            "the margin box counter(page) glyph ('1') should be embedded in the ToUnicode CMap"
        );
    }

    #[test]
    fn counter_pages_in_a_margin_box_is_rejected_in_streaming_mode() {
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        match engine.feed(
            b"<html><head><style>\
                @page { @bottom-center { content: counter(pages); } }\
              </style></head><body><p>x</p></body></html>",
        ) {
            Err(EngineError::UnsupportedInStreamingMode(_)) => {}
            other => panic!("expected UnsupportedInStreamingMode, got {other:?}"),
        }
    }

    #[test]
    fn counter_page_alone_is_allowed_in_streaming_mode() {
        // `counter(page)`単体(`counter(pages)`を伴わない)は、ページ確定時点で
        // 値が決まるためストリーミングでも問題なく動作するはず([0028]決定6)。
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine
            .feed(
                b"<html><head><style>\
                    @page { @bottom-right { content: counter(page); } }\
                  </style></head><body><p>x</p></body></html>",
            )
            .expect("counter(page) alone should be allowed in streaming mode");
        let bytes = engine.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn counter_pages_resolves_to_the_actual_total_page_count_in_batch_mode() {
        // `@page`の`size`/`margin`を明示指定してページ数を決定論的にする:
        // ページ内容領域の高さ=300px(margin 0)、300px高さのdivを2個並べれば
        // ちょうど2ページに分かれるはず。
        let options = EngineOptions {
            mode: Mode::Batch,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        let html = b"<html><head><style>\
               @page { size: 200px 300px; margin: 0; @bottom-right { content: counter(pages); } }\
               body { margin: 0; } div { height: 300px; }\
             </style></head><body><div></div><div></div></body></html>";
        engine.feed(html).unwrap();
        let bytes = engine.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(
            count_occurrences(&bytes, b"/MediaBox [0 0 200 300]"),
            2,
            "expected exactly 2 pages"
        );
        let decompressed = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&decompressed, b"<0032>") > 0,
            "counter(pages) should resolve to the actual total page count ('2') in the ToUnicode CMap"
        );
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

    #[test]
    fn unicode_range_hard_filter_excludes_a_face_end_to_end_through_the_engine() {
        // 1つ目の`@font-face`(index 0)はDejaVu Sansだが`unicode-range:
        // U+0-7F`(Basic Latinのみ)を宣言する。'é'(U+00E9)はDejaVu Sans
        // 自身が実際に描画できるグリフだが、宣言レンジ外なのでハード
        // フィルタで除外されるはず。2つ目の`@font-face`(index 1)は同じ
        // DejaVu Sansをrange指定なしで再登録したもので、こちらが
        // 選ばれるはず。CSSパース→`Engine`→`FontCollection`の実際の
        // パイプラインを通した回帰検知(0011のT39)。
        let base_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts"));
        let html = r#"<html><head><style>
            @font-face { font-family: "Brand"; src: url("DejaVuSans.ttf"); unicode-range: U+0-7F; }
            @font-face { font-family: "Brand"; src: url("DejaVuSans.ttf"); }
            p { font-family: "Brand"; }
        </style></head><body><p>ééé</p></body></html>"#;

        let options = EngineOptions {
            base_dir: Some(base_dir.to_path_buf()),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        let stream = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&stream, b"/F1 ") > 0,
            "should select the unrestricted second face (index 1) for U+00E9"
        );
        assert_eq!(
            count_occurrences(&stream, b"/F0 "),
            0,
            "the range-restricted first face (index 0) should never be selected for U+00E9, \
             even though it physically has the glyph"
        );
    }

    #[test]
    fn unicode_range_split_between_latin_and_cjk_faces_matches_in_batch_and_streaming_mode() {
        // 典型的な「英数字用+CJK用を同一family名でunicode-range分けして
        // 併用」パターン(0004 T38/T39)。`Mode::Batch`/`Mode::Streaming`
        // 両方で同じ結果になることも確認する。
        let base_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts"));
        let html_src = r#"<style>
            @font-face { font-family: "Brand"; src: url("DejaVuSans.ttf"); unicode-range: U+0-24F; }
            @font-face { font-family: "Brand"; src: url("NotoSansCJK-Regular.ttc"); unicode-range: U+4E00-9FFF; }
            p { font-family: "Brand"; }
        </style><body><p>A&#26085;</p></body>"#;

        let run = |mode: Mode| {
            let options = EngineOptions {
                mode,
                base_dir: Some(base_dir.to_path_buf()),
                ..EngineOptions::default()
            };
            let mut engine = Engine::new(options, MemorySink::new());
            engine.feed(html_src.as_bytes()).unwrap();
            engine.finish().unwrap()
        };

        let batch_bytes = run(Mode::Batch);
        let streaming_bytes = run(Mode::Streaming);

        for (label, bytes) in [("batch", &batch_bytes), ("streaming", &streaming_bytes)] {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{label} output should be a valid PDF"
            );
            let stream = decompressed_stream_bytes(bytes);
            assert!(
                count_occurrences(&stream, b"/F0 ") > 0,
                "{label}: the Latin-range face (index 0) should be used for 'A'"
            );
            assert!(
                count_occurrences(&stream, b"/F1 ") > 0,
                "{label}: the CJK-range face (index 1) should be used for U+65E5"
            );
        }
    }

    const JPEG_FIXTURE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient.jpg"
    );
    const PNG_ALPHA_FIXTURE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient_alpha.png"
    );

    fn data_uri(path: &str, mime_type: &str) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let bytes = std::fs::read(path).unwrap();
        format!("data:{mime_type};base64,{}", STANDARD.encode(bytes))
    }

    #[test]
    fn image_data_uri_is_embedded_as_a_dctdecode_xobject_end_to_end() {
        // M5(画像埋め込み)のパイプライン全体(DOM属性抽出→data:URI分類→
        // デコード→box tree→レイアウト→PDF XObject書き出し)を、
        // fetchを一切経由しないdata:URI経由で検証する。
        let html = format!(
            r#"<html><body><img src="{}" width="32" height="24"></body></html>"#,
            data_uri(JPEG_FIXTURE_PATH, "image/jpeg")
        );

        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        // JPEGはデコードせずそのままDCTDecodeフィルタで埋め込む
        // ([0012]の方針)ため、生のJPEGバイト列そのものが出現するはず。
        let jpeg_bytes = std::fs::read(JPEG_FIXTURE_PATH).unwrap();
        assert!(count_occurrences(&bytes, b"/DCTDecode") > 0);
        assert!(
            count_occurrences(&bytes, &jpeg_bytes) > 0,
            "the original JPEG bytes should be embedded verbatim (no re-encode)"
        );
        assert!(count_occurrences(&bytes, b"/Width 32") > 0);
        assert!(count_occurrences(&bytes, b"/Height 24") > 0);
    }

    #[test]
    fn png_with_alpha_data_uri_produces_an_smask_xobject_end_to_end() {
        let html = format!(
            r#"<html><body><img src="{}"></body></html>"#,
            data_uri(PNG_ALPHA_FIXTURE_PATH, "image/png")
        );

        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(
            count_occurrences(&bytes, b"/SMask") > 0,
            "a PNG with an alpha channel should produce an SMask-linked XObject"
        );
        // 内在サイズ(16x16、フィクスチャの実寸)がwidth/height属性なしで
        // そのまま使われているはず。
        assert!(count_occurrences(&bytes, b"/Width 16") > 0);
        assert!(count_occurrences(&bytes, b"/Height 16") > 0);
    }

    /// `object-fit`/`object-position`のE2Eテスト(M9 Phase 2)。詳細設計は
    /// [0030](../../docs/decisions/0030-object-fit-position-design.md)参照。
    /// `object_fit_rect`自体の幾何計算は`pdf/document.rs`の単体テストで
    /// 網羅済みのため、ここでは実際のパイプライン(data:URIデコード→
    /// box tree→レイアウト→PDFエンコード)を通した疎通・クリップ発行の
    /// 確認に絞る。
    fn build_object_fit_pdf(object_fit_css: &str) -> Vec<u8> {
        let html = format!(
            r#"<html><body><img src="{}" style="width: 150px; height: 80px; {}"></body></html>"#,
            data_uri(JPEG_FIXTURE_PATH, "image/jpeg"),
            object_fit_css
        );
        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        bytes
    }

    #[test]
    fn object_fit_cover_and_none_render_valid_pdfs_with_a_single_image_draw_each() {
        for object_fit in ["cover", "contain", "none", "scale-down", "fill"] {
            let bytes = build_object_fit_pdf(&format!("object-fit: {object_fit};"));
            let decompressed = decompressed_stream_bytes(&bytes);
            assert_eq!(
                count_occurrences(&decompressed, b" Do\n"),
                1,
                "object-fit: {object_fit} should draw the image exactly once (no tiling)"
            );
        }
    }

    #[test]
    fn object_fit_always_clips_to_the_content_box_even_for_the_default_fill() {
        // [0030]決定3: `object-fit`の値によらず常にcontent-boxへクリップする
        // (`Fill`は元々ぴったり収まるがno-opとして同じ経路を通る)。クリップ
        // パスの構築(`re` → `W n`)が実際に発行されていることを確認する。
        let bytes = build_object_fit_pdf("");
        let decompressed = decompressed_stream_bytes(&bytes);
        assert_eq!(count_occurrences(&decompressed, b" re\n"), 1);
        assert!(count_occurrences(&decompressed, b"W\n") > 0);
    }

    #[test]
    fn object_fit_cover_and_fill_produce_different_geometry_end_to_end() {
        // intrinsic 32x24 を 150x80 のボックスへ描画する場合、`fill`
        // (非一様に引き伸ばす)と`cover`(アスペクト比を保って拡大・はみ出し分は
        // クリップ)は描画される画像の変換行列(`cm`)が異なるはずなので、
        // コンテンツストリーム全体としてもバイト列が一致しないはず。
        let fill_bytes = decompressed_stream_bytes(&build_object_fit_pdf("object-fit: fill;"));
        let cover_bytes = decompressed_stream_bytes(&build_object_fit_pdf("object-fit: cover;"));
        assert_ne!(fill_bytes, cover_bytes);
    }

    #[test]
    fn object_position_moves_the_image_within_the_content_box_end_to_end() {
        let center_bytes = decompressed_stream_bytes(&build_object_fit_pdf("object-fit: contain;"));
        let right_bottom_bytes = decompressed_stream_bytes(&build_object_fit_pdf(
            "object-fit: contain; object-position: right bottom;",
        ));
        assert_ne!(center_bytes, right_bottom_bytes);
    }

    #[test]
    fn image_rendering_matches_between_batch_and_streaming_mode() {
        let html = format!(
            r#"<html><body><p>before</p><img src="{}" width="32" height="24"><p>after</p></body></html>"#,
            data_uri(JPEG_FIXTURE_PATH, "image/jpeg")
        );

        let run = |mode: Mode| {
            let options = EngineOptions {
                mode,
                fonts: vec![font_spec()],
                ..EngineOptions::default()
            };
            let mut engine = Engine::new(options, MemorySink::new());
            engine.feed(html.as_bytes()).unwrap();
            engine.finish().unwrap()
        };

        let batch_bytes = run(Mode::Batch);
        let streaming_bytes = run(Mode::Streaming);

        for (label, bytes) in [("batch", &batch_bytes), ("streaming", &streaming_bytes)] {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{label} output should be a valid PDF"
            );
            assert!(
                count_occurrences(bytes, b"/DCTDecode") > 0,
                "{label}: image should be embedded"
            );
        }
    }

    #[test]
    fn background_image_on_a_plain_div_is_embedded_as_a_dctdecode_xobject_end_to_end() {
        // M7(T80-83)のパイプライン全体(パース→カスケード→
        // `resolve_background_images`→PDF XObject書き出し)を検証する。
        // `<div>`は`background-color`も枠線も持たない
        // (`has_visible_decoration`がbackground-imageも見るよう修正した
        // 効果を確認する。修正前は装飾フラグメントが生成されず、この
        // 背景画像は`page.boxes`に一切現れなかった)。
        let html = format!(
            r#"<html><body><div style="background-image: url('{}'); width: 32px; height: 24px;"></div></body></html>"#,
            data_uri(JPEG_FIXTURE_PATH, "image/jpeg")
        );

        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        let jpeg_bytes = std::fs::read(JPEG_FIXTURE_PATH).unwrap();
        assert!(count_occurrences(&bytes, b"/DCTDecode") > 0);
        assert!(
            count_occurrences(&bytes, &jpeg_bytes) > 0,
            "the background-image's original JPEG bytes should be embedded verbatim"
        );
    }

    #[test]
    fn background_image_rendering_matches_between_batch_and_streaming_mode() {
        let html = format!(
            r#"<html><body><p>before</p><div style="background-image: url('{}'); width: 32px; height: 24px;"></div><p>after</p></body></html>"#,
            data_uri(JPEG_FIXTURE_PATH, "image/jpeg")
        );

        let run = |mode: Mode| {
            let options = EngineOptions {
                mode,
                fonts: vec![font_spec()],
                ..EngineOptions::default()
            };
            let mut engine = Engine::new(options, MemorySink::new());
            engine.feed(html.as_bytes()).unwrap();
            engine.finish().unwrap()
        };

        let batch_bytes = run(Mode::Batch);
        let streaming_bytes = run(Mode::Streaming);

        for (label, bytes) in [("batch", &batch_bytes), ("streaming", &streaming_bytes)] {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{label} output should be a valid PDF"
            );
            assert!(
                count_occurrences(bytes, b"/DCTDecode") > 0,
                "{label}: background image should be embedded"
            );
        }
    }

    #[test]
    fn a_broken_background_image_url_degrades_gracefully_instead_of_failing_the_whole_document() {
        // [0014]/[0017]の方針: 取得・デコード失敗はその要素の背景画像だけ
        // 空扱いにして、文書生成全体は止めない。
        let html = r#"<html><body><p>before</p>
            <div style="background-image: url('does-not-exist-anywhere.png'); width: 50px; height: 50px;"></div>
            <p>after</p></body></html>"#;

        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine
            .finish()
            .expect("a broken background-image url must not fail the whole document");

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(
            count_occurrences(&bytes, b"/DCTDecode"),
            0,
            "no image XObject should have been written for the failed fetch"
        );
    }

    #[test]
    fn a_broken_image_src_degrades_to_an_empty_box_instead_of_failing_the_whole_document() {
        // [0014]の方針: 取得・デコード失敗はその要素だけ空扱いにして、
        // 文書生成全体は止めない(壊れたURLがDoSベクタにならないように)。
        let html = r#"<html><body><p>before</p>
            <img src="does-not-exist-anywhere.png" width="50" height="50">
            <p>after</p></body></html>"#;

        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine
            .finish()
            .expect("a broken image src must not fail the whole document");

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(
            count_occurrences(&bytes, b"/DCTDecode"),
            0,
            "no image XObject should have been written for the failed fetch"
        );
    }

    #[test]
    fn external_stylesheet_via_link_is_applied_end_to_end() {
        // M6のパイプライン全体(<link>検出→fetch→parse→cascade)を、
        // 実際にfont-sizeの違いとしてPDFコンテンツストリームに現れるかで
        // 検証する。
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-engine-test-{}-external_stylesheet",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.css"), b"p { font-size: 40px; }").unwrap();

        let html = r#"<html><head><link rel="stylesheet" href="main.css"></head>
            <body><p>hello</p></body></html>"#;
        let options = EngineOptions {
            fonts: vec![font_spec()],
            base_dir: Some(dir.clone()),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        let stream = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&stream, b"/F0 40 Tf") > 0,
            "the font-size from the fetched external stylesheet should apply"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn external_stylesheet_matches_between_batch_and_streaming_mode() {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-engine-test-{}-external_stylesheet_parity",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.css"), b"p { font-size: 40px; }").unwrap();

        let html = r#"<html><head><link rel="stylesheet" href="main.css"></head>
            <body><p>hello</p></body></html>"#;

        let run = |mode: Mode| {
            let options = EngineOptions {
                mode,
                fonts: vec![font_spec()],
                base_dir: Some(dir.clone()),
                ..EngineOptions::default()
            };
            let mut engine = Engine::new(options, MemorySink::new());
            engine.feed(html.as_bytes()).unwrap();
            engine.finish().unwrap()
        };

        let batch_bytes = run(Mode::Batch);
        let streaming_bytes = run(Mode::Streaming);

        for (label, bytes) in [("batch", &batch_bytes), ("streaming", &streaming_bytes)] {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{label} output should be a valid PDF"
            );
            let stream = decompressed_stream_bytes(bytes);
            assert!(
                count_occurrences(&stream, b"/F0 40 Tf") > 0,
                "{label}: the fetched external stylesheet's font-size should apply"
            );
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn at_import_inside_an_external_stylesheet_is_applied_end_to_end() {
        // M7のパイプライン全体(<link>のfetch→@importの検出・再帰フェッチ→
        // 展開→parse→cascade)を、実際にfont-sizeの違いとしてPDFコンテンツ
        // ストリームに現れるかで検証する。
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-engine-test-{}-at_import",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.css"), br#"@import url("base.css");"#).unwrap();
        std::fs::write(dir.join("base.css"), b"p { font-size: 40px; }").unwrap();

        let html = r#"<html><head><link rel="stylesheet" href="main.css"></head>
            <body><p>hello</p></body></html>"#;
        let options = EngineOptions {
            fonts: vec![font_spec()],
            base_dir: Some(dir.clone()),
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine.finish().unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        let stream = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&stream, b"/F0 40 Tf") > 0,
            "the font-size from the @import-ed stylesheet should apply"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn at_import_matches_between_batch_and_streaming_mode() {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-engine-test-{}-at_import_parity",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.css"), br#"@import url("base.css");"#).unwrap();
        std::fs::write(dir.join("base.css"), b"p { font-size: 40px; }").unwrap();

        let html = r#"<html><head><link rel="stylesheet" href="main.css"></head>
            <body><p>hello</p></body></html>"#;

        let run = |mode: Mode| {
            let options = EngineOptions {
                mode,
                fonts: vec![font_spec()],
                base_dir: Some(dir.clone()),
                ..EngineOptions::default()
            };
            let mut engine = Engine::new(options, MemorySink::new());
            engine.feed(html.as_bytes()).unwrap();
            engine.finish().unwrap()
        };

        let batch_bytes = run(Mode::Batch);
        let streaming_bytes = run(Mode::Streaming);

        for (label, bytes) in [("batch", &batch_bytes), ("streaming", &streaming_bytes)] {
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{label} output should be a valid PDF"
            );
            let stream = decompressed_stream_bytes(bytes);
            assert!(
                count_occurrences(&stream, b"/F0 40 Tf") > 0,
                "{label}: the @import-ed stylesheet's font-size should apply"
            );
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn streaming_mode_rejects_a_late_link_stylesheet_after_body_starts() {
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<body><p>x</p>").unwrap();

        match engine.feed(br#"<link rel="stylesheet" href="late.css">"#) {
            Err(EngineError::UnsupportedInStreamingMode(_)) => {}
            other => panic!("expected UnsupportedInStreamingMode, got {other:?}"),
        }
    }

    #[test]
    fn streaming_mode_allows_a_late_link_that_is_not_a_stylesheet() {
        // rel="stylesheet"以外のlink(favicon等)は、<body>より後に
        // 出現してもストリーミングモードの制約対象外のはず。
        let options = EngineOptions {
            mode: Mode::Streaming,
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(b"<body><p>x</p>").unwrap();
        engine
            .feed(br#"<link rel="icon" href="favicon.ico">"#)
            .expect("a non-stylesheet <link> after <body> should not be rejected");
    }

    #[test]
    fn a_failed_external_stylesheet_does_not_fail_the_whole_document() {
        // 0015/T66: 外部スタイルシートの取得失敗はそのスタイルシートだけを
        // 無視し、文書生成全体は止めない(画像[0014]と同じ方針)。
        let html = r#"<html><head><link rel="stylesheet" href="does-not-exist.css"></head>
            <body><p>hello</p></body></html>"#;
        let options = EngineOptions {
            fonts: vec![font_spec()],
            ..EngineOptions::default()
        };
        let mut engine = Engine::new(options, MemorySink::new());
        engine.feed(html.as_bytes()).unwrap();
        let bytes = engine
            .finish()
            .expect("a broken external stylesheet must not fail the whole document");

        assert!(bytes.starts_with(b"%PDF-"));
    }
}
