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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::fonts::{
    ensure_cjk_fallback_font, load_font_faces, load_fonts_for_uncovered_chars,
    load_missing_system_fonts, warn_uncovered_chars, Font, FontCollection, SystemFonts,
};
use crate::html::{
    collect_anchor_targets, find_base_href, find_document_title, Dom, NodeData, NodeId,
    StreamingParser,
};
use crate::img::{DocumentImageCache, ImageFetcher};
use crate::layout::{
    build_box_for_element, collect_completed_subtree_roots, has_visible_decoration,
    layout_document_from, paginate_document, paginate_document_with_absolutes,
    resolve_background_images, resolve_border, resolve_images, resolve_lpa_or_zero,
    resolve_padding, resolve_width_and_horizontal_margins, EdgeSizes, LaidOutBox, LaidOutContent,
    PageSettings, Rect, StreamingPaginator,
};
use crate::pdf::{
    anchor_destination_name, ImageAssetCache, LinkSettings, PageOverlay, PdfOutputOptions,
    PreparedImage, StreamingPdfWriter,
};
use crate::sink::Sink;
use crate::style::{
    backward_looking_selectors, compute_single_element_style, compute_styles,
    compute_styles_with_parent, extract_author_stylesheet, resolve_page_rules,
    rules_use_page_count, user_agent_stylesheet, ComputedStyle, LengthPercentageOrAuto, PageRule,
    RgbaColor, Stylesheet,
};
use crate::style::{FontStyle, FontWeight};

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

/// CSSの汎用family名のうち、実体を明示指定できるもの。
/// `cursive`/`fantasy`は対象外(M12決定6)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericFamily {
    SansSerif,
    Serif,
    Monospace,
}

impl GenericFamily {
    /// CSSで書かれる名前。この名前でフォントコレクションへ登録する。
    pub fn css_name(self) -> &'static str {
        match self {
            Self::SansSerif => "sans-serif",
            Self::Serif => "serif",
            Self::Monospace => "monospace",
        }
    }
}

/// `--font`相当の明示的なフォント指定。
pub struct FontSpec {
    pub path: PathBuf,
    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイルのフェイス番号。
    pub index: u32,
}

/// レンダリング内容の挙動を変えるオプション(M12 Phase 4)。
///
/// PDFの書き出し方だけを変える[`crate::pdf::PdfOutputOptions`]と対になる、
/// 「何を描くか」側の設定。
#[derive(Debug, Clone)]
pub struct ContentOptions {
    /// `<img>`とCSS`background-image`を読み込むか(`--no-images`でfalse)。
    pub load_images: bool,
    /// 要素の背景(色・画像)を描くか(`--no-background`でfalse)。
    pub draw_backgrounds: bool,
    /// ユーザーオリジンのCSS(`--user-style-sheet`)。UAスタイルシートの
    /// 後ろへ連結する(UAより強く、著者CSSより弱い位置)。
    pub user_stylesheets: Vec<String>,
    /// 算出`font-size`の下限(`--minimum-font-size`)。
    pub minimum_font_size: Option<f32>,
    /// 外部リンクの注釈を出すか(`--disable-external-links`でfalse)。
    pub external_links: bool,
    /// 内部リンク(`#id`)の注釈を出すか(`--disable-internal-links`でfalse)。
    pub internal_links: bool,
    /// 相対URLの外部リンクを`<base href>`で絶対化せずそのまま書くか
    /// (`--keep-relative-links`でtrue)。
    pub keep_relative_links: bool,
    /// 画像・CSS・フォントの取得に失敗したら中断するか
    /// (`--load-media-error-handling abort`)。
    pub abort_on_media_error: bool,
}

impl Default for ContentOptions {
    fn default() -> Self {
        Self {
            load_images: true,
            draw_backgrounds: true,
            user_stylesheets: Vec::new(),
            minimum_font_size: None,
            external_links: true,
            internal_links: true,
            keep_relative_links: false,
            abort_on_media_error: false,
        }
    }
}

/// `Engine`の初期化オプション。
#[derive(Default)]
pub struct EngineOptions {
    pub mode: Mode,
    pub settings: PageSettings,
    /// `--font`相当の明示的なフォント指定(複数指定可)。
    pub fonts: Vec<FontSpec>,
    /// CSSの汎用family名(`sans-serif`/`serif`/`monospace`)の実体を明示指定する
    /// (`--gothic-font`/`--serif-font`/`--mono-font`相当)。指定した汎用名は
    /// そのフォントで最優先に解決され、未指定の汎用名はシステムフォントの
    /// 候補リスト([`crate::fonts`])で解決する。既定`font-family`(未指定)は
    /// これに関わらず`--font`のフォントへフォールバックする。
    pub generic_fonts: Vec<(GenericFamily, FontSpec)>,
    /// `@font-face`の`src: url(...)`を相対解決する基準ディレクトリ。
    /// 入力がファイルに対応しない場合(Rackボディ等)は`None`でよく、
    /// その場合はカレントディレクトリを基準にする。`<img src>`のローカル
    /// 相対パス解決にも同じ基準ディレクトリを使う。
    pub base_dir: Option<PathBuf>,
    /// 相対参照の解決基準URL(`--base-url`相当)。HTMLに`<base href>`が
    /// あればそちらが優先される([0040](../docs/decisions/0040-base-href-design.md)
    /// が定めるのは文書内の指定であり、この値はその既定を外から与えるもの)。
    /// http(s)のURLを想定し、ローカルディレクトリを基準にしたい場合は
    /// `base_dir`を使う。
    pub base_href: Option<String>,
    /// `<img src>`・`<link rel=stylesheet href>`のhttp(s)絶対URLフェッチを
    /// 許可するか。既定`false`([0013](../docs/decisions/0013-image-fetch-security.md)
    /// の「既定無効・明示オプトイン」方針。[0015](../docs/decisions/0015-external-stylesheet-fetch-design.md)
    /// 決定2により、画像・外部スタイルシート双方をこの1つのフラグで
    /// 統括する)。ローカル相対パス・`data:`URIはこの値に関わらず常に許可する。
    pub allow_remote_assets: bool,
    /// PDF書き出しオプション(メタデータ・圧縮・スケール・グレースケール、
    /// [0057](../docs/decisions/0057-pdf-output-options-design.md))。
    pub output: PdfOutputOptions,
    /// 描画内容の挙動([`ContentOptions`])。
    pub content: ContentOptions,
    /// ローカルファイル参照の可否と許可ディレクトリ
    /// (`--enable/disable-local-file-access`・`--allow`)。
    /// 既定はCLIの従来挙動どおり「許可・ディレクトリ制限なし」。
    pub local_access: LocalAccess,
    /// `--header-html`/`--footer-html`のテンプレート([0058](
    /// ../docs/decisions/0058-header-footer-design.md)決定3)。
    pub header_footer_html: HeaderFooterHtml,
    /// `--cover`のHTML(プレースホルダ展開済み)。
    pub cover_html: Option<String>,
    /// 目次の設定([0059](../docs/decisions/0059-cover-and-toc-design.md))。
    pub toc: TocSettings,
    /// `--page-offset`。TOC・本文のページ番号の起点をずらす(決定1)。
    pub page_offset: usize,
    /// CLIのヘッダー/フッター簡易オプションから合成した`@page`ルール
    /// ([0058](../docs/decisions/0058-header-footer-design.md)決定1)。
    /// **著者CSSのページルールより前**に置かれるため、同じmargin boxを
    /// 著者が宣言していればそちらが勝つ。
    pub extra_page_rules: Vec<PageRule>,
}

/// ローカルファイル参照の許可設定。
#[derive(Debug, Clone)]
pub struct LocalAccess {
    pub allow: bool,
    /// 空でなければ、この配下のファイルだけを読める。
    pub allowed_dirs: Vec<PathBuf>,
}

impl Default for LocalAccess {
    fn default() -> Self {
        Self {
            allow: true,
            allowed_dirs: Vec::new(),
        }
    }
}

/// `--header-html`/`--footer-html`のテンプレート([0058]決定3)。
///
/// 中身は**プレースホルダ展開前**のHTMLテキスト。ページ番号を含む場合は
/// ページごとに展開してレイアウトし直す(決定5)。
#[derive(Debug, Clone, Default)]
pub struct HeaderFooterHtml {
    pub header: Option<String>,
    pub footer: Option<String>,
    /// ページごとに値が変わるプレースホルダ(`[page]`/`[topage]`)の展開値を
    /// 埋めるための、文書単位で決まる値。
    pub placeholders: HeaderFooterPlaceholders,
}

/// プレースホルダの展開値(CLI層の`PlaceholderValues`から詰め替えたもの)。
/// コアがCLI層に依存しないよう、必要な値だけを持つ素朴な型にしている。
#[derive(Debug, Clone, Default)]
pub struct HeaderFooterPlaceholders {
    /// `[page]`/`[topage]`以外を展開済みにしたテキストを作る関数の代わりに、
    /// 展開済みのテンプレートをそのまま受け取る運用にする。
    /// ここにはページ番号だけを差し込むための素材を持つ。
    pub page_token: String,
    pub total_pages_token: String,
}

impl HeaderFooterHtml {
    pub fn is_empty(&self) -> bool {
        self.header.is_none() && self.footer.is_none()
    }

    /// ページ番号のプレースホルダを含むか(含まなければレイアウト結果を
    /// ページ間で使い回せる、[0058]決定5)。
    pub fn depends_on_page(&self) -> bool {
        [self.header.as_deref(), self.footer.as_deref()]
            .into_iter()
            .flatten()
            .any(|html| {
                html.contains(&self.placeholders.page_token)
                    || html.contains(&self.placeholders.total_pages_token)
            })
    }

    /// `[topage]`(総ページ数)を使っているか。`Mode::Streaming`では
    /// 値が定まらないためエラーにする([0058]決定7)。
    pub fn uses_total_pages(&self) -> bool {
        [self.header.as_deref(), self.footer.as_deref()]
            .into_iter()
            .flatten()
            .any(|html| html.contains(&self.placeholders.total_pages_token))
    }

    fn expand(&self, template: &str, page: usize, total_pages: Option<usize>) -> String {
        let total = total_pages.map(|t| t.to_string()).unwrap_or_default();
        template
            .replace(&self.placeholders.page_token, &page.to_string())
            .replace(&self.placeholders.total_pages_token, &total)
    }
}

/// ヘッダー(`top = true`)またはフッター用に、余白領域を基準とした
/// `PageSettings`とクリップ矩形を作る([0058]決定3)。
fn overlay_area(settings: &PageSettings, top: bool) -> (PageSettings, Rect) {
    let size = settings.size;
    let (margin, clip) = if top {
        (
            EdgeSizes {
                top: 0.0,
                right: settings.margin.right,
                bottom: size.height - settings.margin.top,
                left: settings.margin.left,
            },
            Rect {
                x: settings.margin.left,
                y: 0.0,
                width: settings.content_width(),
                height: settings.margin.top,
            },
        )
    } else {
        (
            EdgeSizes {
                top: size.height - settings.margin.bottom,
                right: settings.margin.right,
                bottom: 0.0,
                left: settings.margin.left,
            },
            Rect {
                x: settings.margin.left,
                y: size.height - settings.margin.bottom,
                width: settings.content_width(),
                height: settings.margin.bottom,
            },
        )
    };
    (PageSettings { size, margin }, clip)
}

/// ヘッダー/フッターHTMLを1つ、余白領域向けにレイアウトして
/// [`PageOverlay`]にする。
///
/// 画像は非対応(既知の限界。`ImageAssetCache`を渡していないため
/// `<img>`は空のボックスになる)。テキスト・枠線・背景色は本文と同じ
/// パイプラインで描かれる。
fn layout_overlay(
    html: &str,
    fonts: &FontCollection,
    settings: &PageSettings,
    top: bool,
    fetcher: &ImageFetcher,
    cache: &DocumentImageCache,
) -> Option<PageOverlay> {
    let (area_settings, clip) = overlay_area(settings, top);
    if area_settings.content_height() <= 0.0 || area_settings.content_width() <= 0.0 {
        return None;
    }

    let dom = crate::html::parse(html.as_bytes());
    let ua = user_agent_stylesheet();
    let author = extract_author_stylesheet(&dom, fetcher, cache);
    let styles = compute_styles(&dom, &ua, &author);
    let pages = paginate_document(&dom, &styles, fonts, &area_settings);
    let boxes = pages.into_iter().next().map(|page| page.boxes)?;
    if boxes.is_empty() {
        return None;
    }

    Some(PageOverlay {
        boxes,
        styles,
        settings: area_settings,
        clip,
    })
}

/// ヘッダー/フッターHTML用のフェッチャ。**外部リソースは取得しない**
/// (インラインの`<style>`とテキストだけを対象にする。既知の限界)。
fn overlay_fetcher() -> ImageFetcher {
    ImageFetcher::new(PathBuf::from("."), false).with_local_access(false, Vec::new())
}

/// このページに重ねるヘッダー/フッターのオーバーレイを作る。
#[allow(clippy::too_many_arguments)]
fn build_page_overlays(
    html: &HeaderFooterHtml,
    fonts: &FontCollection,
    settings: &PageSettings,
    page_number: usize,
    total_pages: Option<usize>,
    fetcher: &ImageFetcher,
    cache: &DocumentImageCache,
    cached: &mut Option<Vec<PageOverlay>>,
) -> Vec<PageOverlay> {
    // ページ番号を含まないなら初回のレイアウトを使い回す([0058]決定5)。
    if !html.depends_on_page() {
        if let Some(overlays) = cached.as_ref() {
            return overlays.clone();
        }
    }

    let mut overlays = Vec::new();
    for (template, top) in [(&html.header, true), (&html.footer, false)] {
        let Some(template) = template else { continue };
        let text = html.expand(template, page_number, total_pages);
        if let Some(overlay) = layout_overlay(&text, fonts, settings, top, fetcher, cache) {
            overlays.push(overlay);
        }
    }
    if !html.depends_on_page() {
        *cached = Some(overlays.clone());
    }
    overlays
}

/// 見出しの一覧から目次のHTMLを組み立てる関数([0059]決定2の構造を
/// CLI層(`cli::toc`)が実装して渡す)。
pub type TocHtmlBuilder = Rc<dyn Fn(&[TocHeading]) -> String>;

/// 目次(`--toc`)の設定([0059](../docs/decisions/0059-cover-and-toc-design.md))。
///
/// 見た目に関わる値はCLI層(`cli::toc::TocOptions`)が組み立てたCSS/HTMLへ
/// 反映されるため、コア側は「有効かどうか」と「HTML組み立て関数」だけを持つ。
#[derive(Clone)]
pub struct TocSettings {
    pub enabled: bool,
    /// 見出しの一覧からTOCのHTMLを組み立てる関数。
    /// CLI層が[0059]決定2の構造で実装したものを渡す。
    pub build_html: TocHtmlBuilder,
    /// 見出しから目次へ戻るリンクを張るか(`--enable-toc-back-links`)。
    pub back_links: bool,
}

impl Default for TocSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            build_html: Rc::new(|_| String::new()),
            back_links: false,
        }
    }
}

impl std::fmt::Debug for TocSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TocSettings")
            .field("enabled", &self.enabled)
            .field("back_links", &self.back_links)
            .finish_non_exhaustive()
    }
}

/// 目次に載せる見出し1件([0059]決定5)。
#[derive(Debug, Clone, PartialEq)]
pub struct TocHeading {
    /// `h1`=1 … `h6`=6。
    pub level: u8,
    pub title: String,
    /// 本文内での0始まりのページ番号。表示番号は
    /// `body_page + 1 + TOCページ数 + page_offset`。
    pub body_page: usize,
    /// リンク先の名前付き宛先([0042])。
    pub anchor: String,
}

/// 本文のページ列から`h1`〜`h6`を拾い、そのページ番号とアンカー名を集める。
///
/// `id`を持たない見出しには`__sgtoc_<連番>`を自動で振り、`anchor_names`へ
/// 追加する(決定5)。
fn collect_headings(
    dom: &Dom,
    pages: &[crate::layout::Page],
    anchor_names: &mut HashMap<NodeId, String>,
) -> Vec<TocHeading> {
    fn heading_level(dom: &Dom, node: NodeId) -> Option<u8> {
        let NodeData::Element { name, .. } = &dom.node(node).data else {
            return None;
        };
        match &*name.local {
            "h1" => Some(1),
            "h2" => Some(2),
            "h3" => Some(3),
            "h4" => Some(4),
            "h5" => Some(5),
            "h6" => Some(6),
            _ => None,
        }
    }

    fn text_of(dom: &Dom, node: NodeId, out: &mut String) {
        match &dom.node(node).data {
            NodeData::Text { contents } => out.push_str(contents),
            NodeData::Element { .. } => {
                for child in dom.children(node) {
                    text_of(dom, child, out);
                }
            }
            _ => {}
        }
    }

    fn walk(
        dom: &Dom,
        b: &LaidOutBox,
        page_index: usize,
        seen: &mut Vec<NodeId>,
        out: &mut Vec<(NodeId, u8, usize)>,
    ) {
        if let Some(node) = b.node {
            if let Some(level) = heading_level(dom, node) {
                if !seen.contains(&node) {
                    seen.push(node);
                    out.push((node, level, page_index));
                }
            }
        }
        // 子の辿り方は`pdf::document::collect_link_areas`と同じ構造。
        match &b.content {
            LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
                for child in children {
                    walk(dom, child, page_index, seen, out);
                }
            }
            LaidOutContent::Grid(grid) => {
                for child in grid.rows.iter().flat_map(|row| &row.items) {
                    walk(dom, child, page_index, seen, out);
                }
            }
            LaidOutContent::Table(table) => {
                if let Some(caption) = &table.caption {
                    walk(dom, caption, page_index, seen, out);
                }
                for row in &table.rows {
                    for cell in &row.cells {
                        walk(dom, cell, page_index, seen, out);
                    }
                }
            }
            LaidOutContent::Inline(lines) => {
                for line in lines {
                    for atomic in &line.atomics {
                        walk(dom, &atomic.content, page_index, seen, out);
                    }
                }
            }
            _ => {}
        }
    }

    let mut found: Vec<(NodeId, u8, usize)> = Vec::new();
    let mut seen: Vec<NodeId> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        for b in &page.boxes {
            walk(dom, b, index, &mut seen, &mut found);
        }
    }

    found
        .into_iter()
        .enumerate()
        .map(|(i, (node, level, body_page))| {
            let anchor = match anchor_names.get(&node) {
                Some(existing) => existing.clone(),
                None => {
                    // `id`が無い見出しには自動で宛先名を振る(決定5)。
                    let name = anchor_destination_name(&format!("__sgtoc_{i}"));
                    anchor_names.insert(node, name.clone());
                    name
                }
            };
            let mut title = String::new();
            text_of(dom, node, &mut title);
            TocHeading {
                level,
                title: title.split_whitespace().collect::<Vec<_>>().join(" "),
                body_page,
                anchor,
            }
        })
        .collect()
}

/// 独立したHTMLドキュメント(cover/TOC)をレイアウトしてページ列にする
/// ([0059]決定3)。外部リソースは取得しない([0058]決定3-1と同じ制約)。
fn render_standalone_document(
    html: &str,
    fonts: &FontCollection,
    settings: &PageSettings,
) -> Vec<crate::layout::Page> {
    let dom = crate::html::parse(html.as_bytes());
    let ua = user_agent_stylesheet();
    let author = extract_author_stylesheet(&dom, &overlay_fetcher(), &DocumentImageCache::new());
    let styles = compute_styles(&dom, &ua, &author);
    paginate_document(&dom, &styles, fonts, settings)
}

/// 目次のページ列を、ページ数が収束するまで組み立て直す([0059]決定4)。
///
/// 戻り値は(TOCのページ列, TOCドキュメントのスタイル)。TOCは独立ドキュメント
/// なので、描画にはそのスタイルマップが要る。
fn build_toc_pages(
    headings: &[TocHeading],
    toc: &TocSettings,
    page_offset: usize,
    fonts: &FontCollection,
    settings: &PageSettings,
) -> (Vec<crate::layout::Page>, HashMap<NodeId, ComputedStyle>) {
    const MAX_ROUNDS: usize = 3;

    let mut toc_page_count = 1;
    let mut result = (Vec::new(), HashMap::new());

    for round in 0..MAX_ROUNDS {
        let numbered: Vec<TocHeading> = headings
            .iter()
            .map(|h| TocHeading {
                body_page: h.body_page + 1 + toc_page_count + page_offset,
                ..h.clone()
            })
            .collect();
        let html = (toc.build_html)(&numbered);

        let dom = crate::html::parse(html.as_bytes());
        let ua = user_agent_stylesheet();
        let author =
            extract_author_stylesheet(&dom, &overlay_fetcher(), &DocumentImageCache::new());
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, fonts, settings);

        let converged = pages.len() == toc_page_count;
        toc_page_count = pages.len().max(1);
        result = (pages, styles);
        if converged {
            return result;
        }
        if round + 1 == MAX_ROUNDS {
            eprintln!(
                "警告: 目次のページ数が収束しませんでした(最後の結果を使います)。\n  \
                 目次のページ番号が1ページ分ずれる可能性があります"
            );
        }
    }
    result
}

/// `--font`で明示されたフォントを読む。
fn load_explicit_fonts<E>(specs: &[FontSpec]) -> Result<Vec<Font>, EngineError<E>> {
    let mut loaded = Vec::with_capacity(specs.len());
    for spec in specs {
        let font = Font::load_indexed(&spec.path, spec.index)
            .map_err(|e| EngineError::Font(format!("フォントの読み込みに失敗しました: {e}")))?;
        loaded.push(font);
    }
    Ok(loaded)
}

/// `--font`・`@font-face`・システムフォント探索をすべて終えてもフォントが
/// 1つも無い場合に、システムの`sans-serif`候補を**既定フォント**として補う。
///
/// フォントが1つも無いと、`font-family`未指定のテキスト(既定`font-family`は
/// 空、[0036](../docs/decisions/0036-ua-stylesheet-and-hidden-elements-design.md)
/// 決定3-1改訂)の描画先が無くなる。`--font`を必須にせず**システムフォントで
/// 埋める**ことで、wkhtmltopdfと同じ使い心地にしている(その代わり、
/// 何も指定しなかった場合の出力は実行環境に依存する)。
///
/// `@font-face`でフォントが供給されている場合は**何もしない**。ここで
/// 足してしまうとフェイスの並び順が変わってしまうため。
fn ensure_default_font<E>(
    fonts: &mut FontCollection,
    system: &SystemFonts,
) -> Result<(), EngineError<E>> {
    if !fonts.is_empty() {
        return Ok(());
    }
    match system.load_generic("sans-serif", FontWeight::Normal, FontStyle::Normal) {
        Some(font) => {
            fonts.push_font_face("sans-serif".to_string(), None, None, Vec::new(), font);
            Ok(())
        }
        None => Err(EngineError::Font(
            "使用できるフォントがありません(システムフォントが見つかりませんでした)。\n  \
             --fontでフォントファイルを指定してください"
                .to_string(),
        )),
    }
}

/// `Mode::Streaming`で`font-family`が解決できなかった場合に警告する。
///
/// ストリーミング処理では[`crate::pdf::StreamingPdfWriter`]が`new`の時点で
/// フォント数を固定するため、後から`font-family`名でシステムフォントを
/// 探して足すことができない(`load_missing_system_fonts`を呼べない)。
/// 該当する指定は**黙って既定フォントで描画される**ので、一度だけ警告する。
fn warn_unresolved_font_families(
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    warned: &mut Vec<String>,
) {
    for style in styles.values() {
        for family in &style.font_family {
            if fonts.has_matching_face(family, style.font_weight, style.font_style) {
                continue;
            }
            if warned.iter().any(|f| f == family) {
                continue;
            }
            warned.push(family.clone());
            eprintln!(
                "警告: font-family \"{family}\" はストリーミングモードでは解決できません\n  \
                 (フォントは処理開始時に確定させる必要があるため)。既定のフォントで描画します。\n  \
                 --font/--gothic-font/--serif-font/--mono-font か @font-face で明示してください"
            );
        }
    }
}

/// CLI由来の`@page`ルールを著者ルールの前に並べたものを返す([0058]決定1)。
fn page_rules_with_cli(extra: &[PageRule], author: &[PageRule]) -> Vec<PageRule> {
    let mut rules = extra.to_vec();
    rules.extend_from_slice(author);
    rules
}

/// ユーザーオリジンのCSSをUAスタイルシートの後ろへ連結する。
///
/// CSSのカスケードではユーザーオリジンは「UAより強く著者CSSより弱い」。
/// UAシートの末尾に置けば同オリジン内のソース順で後勝ちになり、著者CSSには
/// 負けるため、この近似で意図した強さになる(`!important`は未対応のため
/// 逆転の問題も起きない)。
fn append_user_stylesheets(ua: &mut Stylesheet, user_css: &[String]) {
    for css in user_css {
        let sheet = crate::style::parse_stylesheet(css);
        ua.rules.extend(sheet.rules);
    }
}

/// スタイル計算後の一括後処理(`--no-background`・`--minimum-font-size`)。
fn apply_content_options(styles: &mut HashMap<NodeId, ComputedStyle>, content: &ContentOptions) {
    for style in styles.values_mut() {
        if !content.draw_backgrounds {
            style.background_color = RgbaColor::TRANSPARENT;
            style.background_image = None;
        }
        if let Some(min) = content.minimum_font_size {
            if style.font_size.0 < min {
                style.font_size.0 = min;
            }
        }
    }
}

/// `Engine`が返すエラー。`Sink`からのエラー(`Io`)、コア自身が判定する
/// 構造エラー(`UnsupportedInStreamingMode`)、フォント読み込みエラー
/// (`Font`)を区別する。
#[derive(Debug)]
pub enum EngineError<E> {
    Io(E),
    UnsupportedInStreamingMode(&'static str),
    Font(String),
    /// `--load-media-error-handling abort`のときに、画像・外部CSS等の
    /// 取得に失敗した(M12 T300)。
    MediaLoad(String),
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
            Self::MediaLoad(msg) => write!(f, "リソースの取得に失敗しました: {msg}"),
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
    /// ページのジオメトリ(オーバーレイの領域計算に使う)。
    page_settings: PageSettings,
    /// ページ番号に依存しないヘッダー/フッターHTMLのレイアウト結果
    /// ([0058]決定5)。
    overlay_cache: Option<Vec<PageOverlay>>,
    /// 解決できない`font-family`について警告済みの名前(同じ警告を
    /// 何度も出さないため)。
    warned_font_families: Vec<String>,
    /// どのフォントでも描画できず警告済みの文字([0065]決定4)。
    /// ストリーミングではトップレベル要素ごとに判定するため、既に警告した
    /// 文字を持ち回って重複を防ぐ。
    warned_uncovered_chars: HashSet<char>,
    paginator: StreamingPaginator,
    writer: StreamingPdfWriter<S>,
    /// `<img>`のフェッチ・デコード結果を文書内でメモ化するキャッシュ。
    image_cache: ImageAssetCache,
}

/// HTMLチャンク投入からPDFバイト列書き出しまでを1つのAPIとして統合する
/// コアのエントリポイント。
/// `--gothic-font`を`font-family: sans-serif`の実体として登録する
/// ([0036]決定3-1改訂)。`push_font_face`で宣言family名`"sans-serif"`として
/// 追加するので、`select_for_char`の通常のfamily一致でそのまま拾える。
/// `has_matching_face("sans-serif", ...)`が真になるため、後段の
/// `load_missing_system_fonts`はシステムのゴシック探索をスキップする。
/// CSSの汎用family名として明示指定されたフォントを、その汎用名で
/// 引けるように登録する。
fn register_generic_fonts<E>(
    fonts: &mut FontCollection,
    generic_fonts: &[(GenericFamily, FontSpec)],
) -> Result<(), EngineError<E>> {
    for (family, spec) in generic_fonts {
        let font = Font::load_indexed(&spec.path, spec.index).map_err(|e| {
            EngineError::Font(format!(
                "{}のフォントの読み込みに失敗しました: {e}",
                family.css_name()
            ))
        })?;
        fonts.push_font_face(family.css_name().to_string(), None, None, Vec::new(), font);
    }
    Ok(())
}

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
        let mut ua = user_agent_stylesheet();
        append_user_stylesheets(&mut ua, &self.options.content.user_stylesheets);
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
        let base_href =
            find_base_href(&self.parser.dom()).or_else(|| self.options.base_href.clone());
        let css_fetcher =
            ImageFetcher::new(base_dir.to_path_buf(), self.options.allow_remote_assets)
                .with_base_href(base_href.clone())
                .with_local_access(
                    self.options.local_access.allow,
                    self.options.local_access.allowed_dirs.clone(),
                );
        let css_cache = DocumentImageCache::new();
        let author = {
            let dom = self.parser.dom();
            extract_author_stylesheet(&dom, &css_fetcher, &css_cache)
        };
        let page_rules = page_rules_with_cli(&self.options.extra_page_rules, &author.page_rules);
        let page_settings = apply_page_rule_settings_override(self.options.settings, &page_rules);
        // `counter(pages)`は文書全体のページ分割完了まで値が定まらないため、
        // 真のストリーミング処理とは原理的に相容れない([0028](
        // ../docs/decisions/0028-paged-media-design.md)決定6、ユーザー確認済み)。
        if rules_use_page_count(&page_rules) {
            return Err(EngineError::UnsupportedInStreamingMode(
                "counter(pages) in @page margin boxes is not supported in streaming mode",
            ));
        }
        // `--header-html`/`--footer-html`の`[topage]`も同じ理由で使えない
        // ([0058]決定7)。
        if self.options.header_footer_html.uses_total_pages() {
            return Err(EngineError::UnsupportedInStreamingMode(
                "[topage] in --header-html/--footer-html is not supported in streaming mode",
            ));
        }
        // 目次は本文全体のページ分割が終わらないと作れない([0059]決定6)。
        if self.options.toc.enabled {
            return Err(EngineError::UnsupportedInStreamingMode(
                "--toc is not supported in streaming mode",
            ));
        }
        // 後方参照セレクタ([0006]分類3)は常に非マッチになる。エラーには
        // しないが、黙って結果が変わるのは避けたいので警告する。
        let backward = backward_looking_selectors(&author);
        if !backward.is_empty() {
            eprintln!(
                "警告: {} はストリーミングモードでは常に非マッチになります\n  \
                 (対象要素の親の子リストが完結するまで判定できないため)。\n  \
                 これらを使う場合は --streaming を外してください",
                backward.join(", ")
            );
        }

        let system_fonts = SystemFonts::scan();
        let mut fonts = FontCollection::new(load_explicit_fonts(&self.options.fonts)?);

        register_generic_fonts(&mut fonts, &self.options.generic_fonts)?;
        for loaded in load_font_faces(&author.font_faces, base_dir, &system_fonts) {
            fonts.push_font_face(
                loaded.family,
                Some(loaded.weight),
                Some(loaded.style),
                loaded.unicode_range,
                loaded.font,
            );
        }
        // `load_missing_system_fonts`・`load_fonts_for_uncovered_chars`は
        // 文書全体のスタイル(や文字)を必要とするが、真のストリーミング処理では
        // 文書全体を一度に持たないため、ここでは呼ばない(モジュールdocの
        // 既知の限界を参照)。代わりに、フォントが何も与えられていない場合は
        // 既定フォント(ラテン)に加えてCJKカバー用のフォントを先回りで足す
        // ([0065](../.claude/plans/decisions/0065-glyph-coverage-font-fallback.md)
        // 決定3)。`--font`/`@font-face`でフォントが供給されている場合に
        // 勝手に足さないのは、フェースの並び順([0011]の`unicode-range`先勝ち)と
        // 「`--font`で渡したフォントが既定になる」原則([0036]決定3-1改訂)への
        // 影響を避けるため。
        let had_no_fonts = fonts.is_empty();
        ensure_default_font(&mut fonts, &system_fonts)?;
        if had_no_fonts {
            ensure_cjk_fallback_font(&mut fonts, &system_fonts);
        }

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

        // `--title`未指定なら`<title>`をPDFの`/Title`に使う([0057]決定6)。
        let mut output = self.options.output.clone();
        output
            .metadata
            .fill_title_from_document(find_document_title(&self.parser.dom()));

        let writer = StreamingPdfWriter::with_options(
            &fonts,
            page_settings,
            sink,
            page_rules.clone(),
            LinkSettings {
                anchor_names: anchor_names.clone(),
                base_href: base_href.clone(),
                external: self.options.content.external_links,
                internal: self.options.content.internal_links,
                keep_relative: self.options.content.keep_relative_links,
            },
            output,
        )
        .map_err(EngineError::Io)?;
        let image_cache = ImageAssetCache::with_fetcher(
            ImageFetcher::new(base_dir.to_path_buf(), self.options.allow_remote_assets)
                .with_base_href(base_href)
                .with_local_access(
                    self.options.local_access.allow,
                    self.options.local_access.allowed_dirs.clone(),
                ),
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
            page_settings,
            overlay_cache: None,
            warned_font_families: Vec::new(),
            warned_uncovered_chars: HashSet::new(),
            paginator: StreamingPaginator::new(page_settings.content_height()),
            writer,
            image_cache,
        })
    }

    /// 確定した1つのトップレベル要素(`<body>`直下の子)を、スタイル計算・
    /// レイアウト・ページ分割・PDF書き出し・DOM解放まで処理する。
    fn process_top_level_element(&mut self, node: NodeId) -> Result<(), EngineError<S::Error>> {
        let Engine {
            parser,
            streaming,
            options,
            ..
        } = self;
        let options_content = &options.content;
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
            let mut sub_styles = sub_styles;
            apply_content_options(&mut sub_styles, options_content);
            warn_unresolved_font_families(
                &sub_styles,
                &state.fonts,
                &mut state.warned_font_families,
            );
            // ストリーミングでは文字ベースのフォント補完ができない
            // ([0065]決定3)ので、描画できない文字が出たら都度警告する。
            warn_uncovered_chars(
                &state.fonts,
                &dom,
                &sub_styles,
                &mut state.warned_uncovered_chars,
            );
            let mut item_box = build_box_for_element(&dom, &sub_styles, node);
            if let (Some(item_box), true) = (&mut item_box, options_content.load_images) {
                resolve_images(item_box, &dom, &state.image_cache);
            }
            (sub_styles, item_box)
        };
        if options_content.load_images {
            state
                .background_images
                .extend(resolve_background_images(&sub_styles, &state.image_cache));
        }
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
            if !options.header_footer_html.is_empty() {
                let page_number = state.writer.page_count() + 1;
                // `Mode::Streaming`では総ページ数が不明なので`[topage]`は空になる
                // ([0058]決定7)。
                let overlays = build_page_overlays(
                    &options.header_footer_html,
                    &state.fonts,
                    &state.page_settings,
                    page_number,
                    None,
                    &overlay_fetcher(),
                    &DocumentImageCache::new(),
                    &mut state.overlay_cache,
                );
                state.writer.set_page_overlays(overlays);
            }
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
                    image_cache,
                    page_settings,
                    mut overlay_cache,
                    ..
                } = state;
                if self.options.content.abort_on_media_error {
                    if let Some(err) = image_cache.had_errors() {
                        return Err(EngineError::MediaLoad(err));
                    }
                }
                for page in paginator.finish() {
                    if !self.options.header_footer_html.is_empty() {
                        let page_number = writer.page_count() + 1;
                        let overlays = build_page_overlays(
                            &self.options.header_footer_html,
                            &fonts,
                            &page_settings,
                            page_number,
                            None,
                            &overlay_fetcher(),
                            &DocumentImageCache::new(),
                            &mut overlay_cache,
                        );
                        writer.set_page_overlays(overlays);
                    }
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

        let system_fonts = SystemFonts::scan();
        let mut fonts = FontCollection::new(load_explicit_fonts(&options.fonts)?);

        let mut ua = user_agent_stylesheet();
        append_user_stylesheets(&mut ua, &options.content.user_stylesheets);
        let base_dir = options
            .base_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."));
        let base_href = find_base_href(&dom).or_else(|| options.base_href.clone());
        let css_fetcher = ImageFetcher::new(base_dir.to_path_buf(), options.allow_remote_assets)
            .with_base_href(base_href.clone())
            .with_local_access(
                options.local_access.allow,
                options.local_access.allowed_dirs.clone(),
            );
        let css_cache = DocumentImageCache::new();
        let author = extract_author_stylesheet(&dom, &css_fetcher, &css_cache);
        let mut styles = compute_styles(&dom, &ua, &author);
        apply_content_options(&mut styles, &options.content);
        // `<a href="#id">`の宛先候補([0042]決定4)。
        let mut anchor_names: HashMap<NodeId, String> = collect_anchor_targets(&dom)
            .into_iter()
            .map(|(node, id)| (node, anchor_destination_name(&id)))
            .collect();
        let page_rules = page_rules_with_cli(&options.extra_page_rules, &author.page_rules);
        let page_settings = apply_page_rule_settings_override(options.settings, &page_rules);

        register_generic_fonts(&mut fonts, &options.generic_fonts)?;
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
        // family名では手掛かりにならない文字(`font-family`未指定の日本語など)を
        // 文字カバレッジから補う([0065]決定2)。`ensure_default_font`より先に
        // 呼ぶ必要はないが、既定フォントを足す前に文書由来のフォントを
        // 揃えておく方がフェースの並びが読みやすい。
        load_fonts_for_uncovered_chars(&mut fonts, &dom, &styles, &system_fonts);
        ensure_default_font(&mut fonts, &system_fonts)?;
        // 補ってもなお描画できない文字が残っていれば警告する([0065]決定4)。
        warn_uncovered_chars(&fonts, &dom, &styles, &mut HashSet::new());

        let mut output = options.output.clone();
        output
            .metadata
            .fill_title_from_document(find_document_title(&dom));

        let image_cache = ImageAssetCache::with_fetcher(
            ImageFetcher::new(base_dir.to_path_buf(), options.allow_remote_assets)
                .with_base_href(base_href.clone())
                .with_local_access(
                    options.local_access.allow,
                    options.local_access.allowed_dirs.clone(),
                ),
        );
        // `background-image`はレイアウトのサイズ計算に影響しない描画専用の
        // 情報なので、`resolve_images`(box tree構築)とは独立に、文書全体の
        // `styles`から一度だけ構築できる([0017]決定2)。
        let background_images = if options.content.load_images {
            resolve_background_images(&styles, &image_cache)
        } else {
            HashMap::new()
        };

        // `Mode::Batch`は全ページを確定させてから絶対配置([0049](
        // ../docs/decisions/0049-absolute-fixed-positioning-design.md))を
        // オーバーレイし、順に書き出す。`fixed`の全ページ複製・`absolute`の
        // 祖先ページ解決が全ページ確定後でないとできないため、
        // `paginate_document_streaming`(逐次解放)ではなくこちらを使う。
        //
        // cover/TOC([0059](../docs/decisions/0059-cover-and-toc-design.md))の
        // ために、**writerを作る前に**本文のページを確定させる。見出しへ
        // 自動で振るアンカー名を`LinkSettings`へ載せる必要があるため。
        let pages = paginate_document_with_absolutes(
            &mut dom,
            &styles,
            &fonts,
            &page_settings,
            &image_cache,
        );

        // 目次用の見出し収集([0059]決定5)。`id`が無い見出しには自動で
        // 宛先名を振り、`anchor_names`へ足す。
        let headings = if options.toc.enabled {
            collect_headings(&dom, &pages, &mut anchor_names)
        } else {
            Vec::new()
        };

        // 表紙は独立したドキュメントとして先に組み立てる(決定3)。
        let cover_pages = match &options.cover_html {
            Some(html) => render_standalone_document(html, &fonts, &page_settings),
            None => Vec::new(),
        };

        // 目次は「自身のページ数が本文のページ番号をずらす」ため、
        // ページ数が収束するまで最大3回組み立て直す(決定4)。
        let (toc_pages, toc_styles) = if options.toc.enabled {
            build_toc_pages(
                &headings,
                &options.toc,
                options.page_offset,
                &fonts,
                &page_settings,
            )
        } else {
            (Vec::new(), HashMap::new())
        };

        // `counter(pages)`の総ページ数はcoverを除いた「TOC + 本文」
        // ([0059]決定1)。本文のページ分割はすでに済んでいるので、
        // [0028]決定6の事前カウント用パスはもう要らない。
        let total_pages = if rules_use_page_count(&page_rules) {
            Some(toc_pages.len() + pages.len())
        } else {
            None
        };

        let mut writer = StreamingPdfWriter::with_options(
            &fonts,
            page_settings,
            sink,
            page_rules.clone(),
            LinkSettings {
                anchor_names: anchor_names.clone(),
                base_href: base_href.clone(),
                external: options.content.external_links,
                internal: options.content.internal_links,
                keep_relative: options.content.keep_relative_links,
            },
            output,
        )
        .map_err(EngineError::Io)?;

        // 書き出し順は cover → TOC → 本文(決定3)。ページ番号はcoverを
        // 数えず、TOCから`1 + --page-offset`で始める(決定1)。
        let empty_styles: HashMap<NodeId, ComputedStyle> = HashMap::new();
        let empty_images: HashMap<NodeId, Rc<PreparedImage>> = HashMap::new();

        for page in &cover_pages {
            // 番号を持たないページ: margin box・ヘッダー/フッターを出さない。
            writer.set_next_page_number(None);
            writer
                .write_page(page, &empty_styles, &empty_images, &fonts, total_pages)
                .map_err(EngineError::Io)?;
        }

        let mut page_number = 1 + options.page_offset;
        for page in &toc_pages {
            writer.set_next_page_number(Some(page_number));
            writer
                .write_page(page, &toc_styles, &empty_images, &fonts, total_pages)
                .map_err(EngineError::Io)?;
            page_number += 1;
        }

        let mut overlay_cache: Option<Vec<PageOverlay>> = None;
        for page in pages.iter() {
            if !options.header_footer_html.is_empty() {
                let overlays = build_page_overlays(
                    &options.header_footer_html,
                    &fonts,
                    &page_settings,
                    page_number,
                    total_pages,
                    &overlay_fetcher(),
                    &DocumentImageCache::new(),
                    &mut overlay_cache,
                );
                writer.set_page_overlays(overlays);
            }
            writer.set_next_page_number(Some(page_number));
            writer
                .write_page(page, &styles, &background_images, &fonts, total_pages)
                .map_err(EngineError::Io)?;
            page_number += 1;
        }

        if options.content.abort_on_media_error {
            if let Some(err) = image_cache.had_errors().or_else(|| css_cache.had_errors()) {
                return Err(EngineError::MediaLoad(err));
            }
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
            crate::style::LengthPercentage::Calc { px, percent } => px + basis * percent,
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

    /// `/MediaBox`の期待値を**CSS px**で書けるようにするヘルパ([0057])。
    fn media_box(width_px: f32, height_px: f32) -> String {
        format!(
            "/MediaBox [0 0 {} {}]",
            width_px * crate::pdf::DEFAULT_SCALE,
            height_px * crate::pdf::DEFAULT_SCALE
        )
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
            count_occurrences(&bytes, media_box(300.0, 400.0).as_bytes()) > 0,
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
            count_occurrences(&bytes, media_box(300.0, 400.0).as_bytes()) > 0,
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
            count_occurrences(&bytes, media_box(200.0, 300.0).as_bytes()),
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
