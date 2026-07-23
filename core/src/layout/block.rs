//! Block Formatting Context: containing blockに基づく幅計算と、
//! ブロック要素の縦積み配置(CSS2.1 §10.3.3, §9.4.1の簡略版)。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - マージン相殺(margin collapsing)は隣接兄弟間のみ対応する(CSS2.1 §8.3.1)。
//!   親子間の相殺(親の上/下マージンと最初/最後の子のマージンの相殺)、および
//!   高さ0・border/paddingなしの空ブロックを上下マージンが素通りする相殺は
//!   未対応
//! - `direction: rtl`は未対応(常にltr前提。over-constrained時に再計算する辺は
//!   margin-right固定)
//! - 高さのパーセンテージ指定はcontaining blockの高さが不定なため`auto`として扱う
//! - インラインコンテンツの行分割・実際の行数に応じた高さはT6([`super::inline`])が担う
use std::collections::HashMap;
use std::rc::Rc;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::pdf::PreparedImage;
use crate::style::{
    BorderCollapse, BorderStyle, BoxSizing, BreakBetween, BreakInside, CaptionSide, Clear,
    ComputedStyle, Display, Float, Length, LengthPercentage, LengthPercentageOrAuto, Position,
};

use super::box_tree::{BoxContent, ImageBoxContent, LayoutBox};
use super::float_ctx::FloatContext;
use super::geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
use super::inline::{layout_inline_content, shape_run, LineBox};
use super::table::layout_table;

/// マーカー(`list-style-position: outside`)と内容のcontent edgeの間の固定の隙間(px)。
/// ([0022](../../../docs/decisions/0022-list-style-design.md)決定4)。
const LIST_MARKER_GAP: f32 = 8.0;

#[derive(Debug, Clone)]
pub struct LaidOutBox {
    pub node: Option<NodeId>,
    pub layout: Layout,
    /// このボックスの`break-before`/`break-after`/`break-inside`/`orphans`/`widows`の
    /// 計算値(ページ分割の判断にのみ使う。無名ボックスは`ComputedStyle`の
    /// 初期値=`auto`/`auto`/`auto`/2/2)。
    pub fragmentation: FragmentationHints,
    /// このボックスが実際に描画される背景色・枠線を持つか。
    ///
    /// `paginate.rs`が、ページをまたいで分割されるコンテナの装飾フラグメント
    /// (背景・枠線の再現、モジュールdoc参照)を生成する必要があるかどうかの
    /// 判断に使う。`border-radius`の有無はここでは無関係(角丸があっても
    /// 背景色・枠線が両方なければ何も描画されないため、`pdf::document`側の
    /// 描画ロジックとは独立に判定してよい)。装飾フラグメント自体・行の
    /// 合成ラッパーなど無名ボックスは常に`false`(それ自体が再帰的に装飾
    /// フラグメントを持つことはない)。
    pub has_visible_decoration: bool,
    /// `float: left/right`が指定されている要素かどうか。`paginate.rs`が
    /// フロー外要素として特別扱いする判定に使う
    /// ([0019](../../../docs/decisions/0019-float-clear-position-relative-design.md)
    /// 決定3/決定5)。
    pub is_float: bool,
    pub content: LaidOutContent,
    /// `display: list-item`のマーカー(箇条書きの記号・番号)。シェイピング済み
    /// `TextRun`1つを持つ`LineBox`として表現し、`pdf::document::render_line`を
    /// そのまま再利用して描画する([0022](
    /// ../../../docs/decisions/0022-list-style-design.md)決定4)。ページ分割で
    /// このボックスが複数ページにまたがる場合、先頭フラグメントにのみ残す
    /// (`paginate.rs`)。
    pub marker: Option<LineBox>,
}

/// [`LaidOutBox`]が持つCSS Fragmentation関連の計算値。ページ分割(`paginate.rs`)が
/// どこで分割するかを決める際に参照する(レイアウト自体には影響しない)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FragmentationHints {
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    pub break_inside: BreakInside,
    pub orphans: u32,
    pub widows: u32,
}

impl From<&ComputedStyle> for FragmentationHints {
    fn from(style: &ComputedStyle) -> Self {
        Self {
            break_before: style.break_before,
            break_after: style.break_after,
            break_inside: style.break_inside,
            orphans: style.orphans,
            widows: style.widows,
        }
    }
}

impl Default for FragmentationHints {
    fn default() -> Self {
        Self::from(&ComputedStyle::default())
    }
}

#[derive(Debug, Clone)]
pub enum LaidOutContent {
    Blocks(Vec<LaidOutBox>),
    Inline(Vec<LineBox>),
    Table(LaidOutTable),
    /// `<img>`。フェッチ・デコードに失敗していれば`None`
    /// (空の置換要素として扱い、何も描画しない)。
    Image(Option<Rc<PreparedImage>>),
}

/// レイアウト済みのテーブル全体(任意のcaption+行の並び)。
#[derive(Debug, Clone)]
pub struct LaidOutTable {
    /// `Box`は`LaidOutBox`→`LaidOutContent::Table`→`LaidOutTable`の再帰を
    /// 間接参照で断ち切るために必要(`box_tree::TableBox.caption`と同じ理由)。
    pub caption: Option<Box<LaidOutBox>>,
    pub caption_side: CaptionSide,
    pub rows: Vec<LaidOutTableRow>,
}

/// レイアウト済みのテーブル行1行分。
#[derive(Debug, Clone)]
pub struct LaidOutTableRow {
    pub node: NodeId,
    pub cells: Vec<LaidOutBox>,
}

/// ページ幅を初期containing blockとして、ボックスツリー全体をレイアウトする。
pub fn layout_document(
    root: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    page_width: f32,
) -> LaidOutBox {
    layout_document_from(root, styles, fonts, page_width, 0.0, 0.0)
}

/// [`layout_document`]のバリアント: 原点`(0.0, 0.0)`からではなく、
/// `(start_x, start_y)`からレイアウトを開始する。
///
/// マイルストーン3のストリーミング処理で、`<body>`直下のトップレベル要素を
/// 1つずつレイアウトする際、前の要素までの累積高さを`start_y`として渡す
/// ことで、複数回の呼び出しにまたがって「上から下に流れる」通常のブロック
/// レイアウトを継続する。`start_x`/`containing_width`には、`<body>`自身の
/// `margin`/`border`/`padding`を反映した値を渡すことを想定する(`<body>`
/// 自体は個々のトップレベル要素とは別に扱われ、その内側がこの関数の
/// containing blockになるため)。
pub fn layout_document_from(
    root: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    start_x: f32,
    start_y: f32,
) -> LaidOutBox {
    // `layout_document`/`layout_document_from`1回の呼び出し全体で1つの
    // `FloatContext`を共有する([0019]決定1)。
    let mut float_ctx = FloatContext::new();
    layout_box(
        root,
        styles,
        fonts,
        containing_width,
        &mut float_ctx,
        start_x,
        start_y,
    )
}

/// `<caption>`(通常のwidth解決を経る、`table.rs`のcaption配置専用)や
/// block.rs内部の再帰呼び出しで使う。
pub(super) fn layout_box(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    float_ctx: &mut FloatContext,
    x: f32,
    y: f32,
) -> LaidOutBox {
    layout_box_impl(b, styles, fonts, containing_width, None, float_ctx, x, y)
}

/// テーブルセルなど、通常の`width`解決(auto/margin計算)を経ずに
/// content-boxの幅を直接指定してレイアウトしたい場合に使う
/// ([`super::table`]専用)。
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_box_with_forced_width(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    forced_content_width: f32,
    float_ctx: &mut FloatContext,
    x: f32,
    y: f32,
) -> LaidOutBox {
    layout_box_impl(
        b,
        styles,
        fonts,
        containing_width,
        Some(forced_content_width),
        float_ctx,
        x,
        y,
    )
}

/// `b`のcontent幅・margin・padding・borderを解決する(置換要素のauto-size適用込み)。
/// `layout_box_impl`本体と、float配置のための事前幅計算(`layout_float_child`)の
/// 両方から呼ばれる共通ロジック。
fn resolve_box_geometry(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    containing_width: f32,
    forced_content_width: Option<f32>,
) -> (ComputedStyle, EdgeSizes, EdgeSizes, EdgeSizes, f32) {
    let mut style = box_style(b, styles);
    if let BoxContent::Image(image_content) = &b.content {
        apply_replaced_element_auto_size(&mut style, image_content);
    }

    let padding = resolve_padding(&style, containing_width);
    let border = resolve_border(&style);
    let (content_width, margin_left, margin_right) = match forced_content_width {
        Some(w) => (
            w,
            resolve_lpa_or_zero(style.margin_left, containing_width),
            resolve_lpa_or_zero(style.margin_right, containing_width),
        ),
        // floatが明示`width`を持つ場合は`resolve_width_and_horizontal_margins`を
        // 使わない: あの関数の「over-constrained」規則(width/margin-left/
        // margin-right全てが非auto=`margin`省略時のデフォルト0も含むときに
        // margin-rightを残り幅いっぱいに再計算する、CSS2.1 §10.3.3の通常フロー
        // 用ルール)を素通しすると、再計算後の巨大なmargin-rightが
        // `margin_box_width`(float配置計算に使う占有幅)に混入してしまう。
        // floatにはこの再計算規則が無い(CSS2.1 §10.3.5、auto marginは単純に0)
        // ため、ここでは迂回する([0019]決定4)。
        None if style.float != Float::None
            && !matches!(style.width, LengthPercentageOrAuto::Auto) =>
        {
            let width = resolve_lpa_or_zero(style.width, containing_width);
            // `box-sizing: border-box`の場合の変換([0027]決定2)。通常フロー用の
            // `resolve_width_and_horizontal_margins`と同じ調整をここでも行う。
            let width = if style.box_sizing == BoxSizing::BorderBox {
                (width - padding.left - padding.right - border.left - border.right).max(0.0)
            } else {
                width
            };
            (
                width,
                resolve_lpa_or_zero(style.margin_left, containing_width),
                resolve_lpa_or_zero(style.margin_right, containing_width),
            )
        }
        None => resolve_width_and_horizontal_margins(
            &style,
            containing_width,
            padding.left + padding.right,
            border.left + border.right,
        ),
    };
    let margin = EdgeSizes {
        top: resolve_lpa_or_zero(style.margin_top, containing_width),
        right: margin_right,
        bottom: resolve_lpa_or_zero(style.margin_bottom, containing_width),
        left: margin_left,
    };

    (style, padding, border, margin, content_width)
}

#[allow(clippy::too_many_arguments)]
fn layout_box_impl(
    b: &LayoutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    forced_content_width: Option<f32>,
    float_ctx: &mut FloatContext,
    x: f32,
    y: f32,
) -> LaidOutBox {
    let (style, padding, border, margin, content_width) =
        resolve_box_geometry(b, styles, containing_width, forced_content_width);

    let content_x = x + margin.left + border.left + padding.left;
    let content_y = y + margin.top + border.top + padding.top;

    let (content, content_height) = match &b.content {
        BoxContent::Blocks(children) => {
            let mut cursor_y = content_y;
            let mut max_float_bottom = content_y;
            let mut laid_children: Vec<LaidOutBox> = Vec::with_capacity(children.len());
            for child in children {
                let child_style = box_style(child, styles);

                if child_style.clear != Clear::None {
                    cursor_y = float_ctx.clearance(child_style.clear, cursor_y);
                }

                if child_style.float != Float::None {
                    // floatはフローに参加しない(CSS2.1 9.5): マージン相殺の対象外、
                    // `cursor_y`は進めない。`float_ctx`は子・孫にも共有されるため、
                    // このBFC内の以降の通常フロー・インラインコンテンツから
                    // 回り込み判定に見える([0019]決定1/決定3)。
                    let child_laid = layout_float_child(
                        child,
                        &child_style,
                        styles,
                        fonts,
                        content_width,
                        float_ctx,
                        content_x,
                        cursor_y,
                    );
                    let float_top = child_laid.layout.content.y
                        - child_laid.layout.padding.top
                        - child_laid.layout.border.top
                        - child_laid.layout.margin.top;
                    max_float_bottom =
                        max_float_bottom.max(float_top + child_laid.layout.margin_box_height());
                    laid_children.push(child_laid);
                    continue;
                }

                let child_margin_top = resolve_lpa_or_zero(child_style.margin_top, content_width);

                // 隣接兄弟間のマージン相殺(CSS2.1 §8.3.1)。前の兄弟のmargin-bottomと
                // この子のmargin-topを、単純な加算ではなく「正の最大値+負の最小値」
                // で相殺した1つの間隔に置き換える。floatはフローに参加しないため
                // 対象外(直前の非float子を探す)。
                if let Some(prev) = laid_children.iter().rev().find(|c| !c.is_float) {
                    let prev_margin_bottom = prev.layout.margin.bottom;
                    let collapsed = collapse_adjacent_margins(prev_margin_bottom, child_margin_top);
                    cursor_y -= prev_margin_bottom + child_margin_top - collapsed;
                }

                let child_laid = layout_box(
                    child,
                    styles,
                    fonts,
                    content_width,
                    float_ctx,
                    content_x,
                    cursor_y,
                );
                cursor_y += child_laid.layout.margin_box_height();
                laid_children.push(child_laid);
            }
            // 直接の子floatが通常フローより下に伸びていれば、その分だけ
            // auto-heightを拡張する(CSS2.1 10.6.7の浅い実装、孫要素には
            // 伝播しない、既知の簡略化。[0019]参照)。
            let auto_height = cursor_y.max(max_float_bottom) - content_y;
            let height = resolve_height(
                &style,
                padding.top + padding.bottom,
                border.top + border.bottom,
            )
            .unwrap_or(auto_height);
            (LaidOutContent::Blocks(laid_children), height)
        }
        BoxContent::Inline(spans) => {
            let lines = layout_inline_content(
                spans,
                styles,
                fonts,
                content_width,
                content_x,
                content_y,
                Some(&*float_ctx),
            );
            let lines_height: f32 = lines.iter().map(|line| line.rect.height).sum();
            let height = resolve_height(
                &style,
                padding.top + padding.bottom,
                border.top + border.bottom,
            )
            .unwrap_or(lines_height);
            (LaidOutContent::Inline(lines), height)
        }
        BoxContent::Table(table) => {
            // `display: table`のセルは新しいBlock Formatting Contextを確立する
            // (CSS2.1 9.4.1)ため、外側の`float_ctx`とは独立させる([0019]決定1)。
            // `border-spacing`は`border-collapse: collapse`とは排他([0021]決定1)
            // なので、collapseの場合はここで0に潰してから渡す。
            let (h_spacing, v_spacing) = if style.border_collapse == BorderCollapse::Collapse {
                (0.0, 0.0)
            } else {
                (
                    style.border_spacing_horizontal.0,
                    style.border_spacing_vertical.0,
                )
            };
            let (laid_table, table_height) = layout_table(
                table,
                styles,
                fonts,
                content_width,
                style.table_layout,
                h_spacing,
                v_spacing,
                content_x,
                content_y,
            );
            let height = resolve_height(
                &style,
                padding.top + padding.bottom,
                border.top + border.bottom,
            )
            .unwrap_or(table_height);
            (LaidOutContent::Table(laid_table), height)
        }
        BoxContent::Image(image_content) => {
            // `apply_replaced_element_auto_size`が呼ばれた場合、widthが両方
            // autoだったケースは既に具体的なLengthへ差し替え済みなので、
            // `resolve_height`は`Some`を返す(高さゼロは、内在サイズが
            // 得られない=フェッチ・デコード失敗時の妥当な既定)。
            let height = resolve_height(
                &style,
                padding.top + padding.bottom,
                border.top + border.bottom,
            )
            .unwrap_or(0.0);
            (LaidOutContent::Image(image_content.image.clone()), height)
        }
    };

    // `position: relative`の視覚的オフセット。後続兄弟の`cursor_y`計算は
    // `margin_box_height()`(座標に依存しない)を使うため、ここでcontent座標を
    // ずらしても後続要素のフローには影響しない([0019]決定6)。
    let (offset_x, offset_y) = if style.position == Position::Relative {
        resolve_relative_offset(&style, content_width)
    } else {
        (0.0, 0.0)
    };

    let marker = b.marker.as_deref().and_then(|text| {
        layout_list_marker(
            text,
            &style,
            fonts,
            content_x + offset_x,
            content_y + offset_y,
        )
    });

    LaidOutBox {
        node: b.node,
        layout: Layout {
            content: Rect {
                x: content_x + offset_x,
                y: content_y + offset_y,
                width: content_width,
                height: content_height,
            },
            padding,
            border,
            margin,
            fragment: FragmentPosition::Whole,
        },
        fragmentation: FragmentationHints::from(&style),
        has_visible_decoration: has_visible_decoration(&style, &border),
        is_float: style.float != Float::None,
        content,
        marker,
    }
}

/// `display: list-item`のマーカー(`list-style-position: outside`、または
/// ブロック子を持つため`inside`からフォールバックした場合)をレイアウトする。
/// マーカーはcontent boxの外側(左のgutter)に独立して配置するだけなので、
/// `b`の内容が`BoxContent::Inline`/`Blocks`のどちらでも同じロジックで扱える
/// ([0022](../../../docs/decisions/0022-list-style-design.md)決定4)。
///
/// 実装は通常のテキストランと全く同じシェイピング(`shape_run`)を再利用し、
/// 結果を`runs`が1つだけの`LineBox`として返す。これにより描画側
/// (`pdf::document::render_line`)を一切変更せずに再利用できる。
fn layout_list_marker(
    text: &str,
    style: &ComputedStyle,
    fonts: &FontCollection,
    content_x: f32,
    content_y: f32,
) -> Option<LineBox> {
    let first_char = text.chars().next()?;
    let font_index = fonts.select_for_char(
        &style.font_family,
        style.font_weight,
        style.font_style,
        first_char,
    )?;
    let run = shape_run(text, font_index, fonts, style);
    let width = run.width;
    Some(LineBox {
        rect: Rect {
            x: content_x - LIST_MARKER_GAP - width,
            y: content_y,
            width,
            height: run.line_height,
        },
        runs: vec![run],
    })
}

/// float子要素を配置する。幅解決は`resolve_box_geometry`で(実際のレイアウトと)
/// 二重に行う——`float_ctx.place`が配置座標を決めるにはmargin box幅が先に
/// 必要なため([`<img>`]のような置換要素のauto-size解決も含めて正確な幅を
/// 得る必要があり、事前計算を省略できない)。
#[allow(clippy::too_many_arguments)]
fn layout_float_child(
    child: &LayoutBox,
    child_style: &ComputedStyle,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    float_ctx: &mut FloatContext,
    containing_left: f32,
    preferred_top: f32,
) -> LaidOutBox {
    let (_, padding, border, margin, child_content_width) =
        resolve_box_geometry(child, styles, containing_width, None);
    let margin_box_width = margin.left
        + border.left
        + padding.left
        + child_content_width
        + padding.right
        + border.right
        + margin.right;

    let (float_x, float_y) = float_ctx.place(
        child_style.float,
        preferred_top,
        containing_left,
        containing_left + containing_width,
        margin_box_width,
    );

    let child_laid = layout_box(
        child,
        styles,
        fonts,
        containing_width,
        float_ctx,
        float_x,
        float_y,
    );
    float_ctx.register(
        child_style.float,
        float_x,
        float_y,
        margin_box_width,
        child_laid.layout.margin_box_height(),
    );
    child_laid
}

/// `position: relative`のtop/right/bottom/leftから視覚的オフセット`(dx, dy)`を
/// 解決する。優先順位はCSS仕様通り`top` > `bottom`、`left` > `right`。
/// `top`/`bottom`のパーセンテージ指定はcontaining blockの高さが不定なため`0`を
/// 基準に解決する(既知の簡略化)。
fn resolve_relative_offset(style: &ComputedStyle, containing_width: f32) -> (f32, f32) {
    let resolve =
        |primary: LengthPercentageOrAuto, secondary: LengthPercentageOrAuto, basis: f32| {
            match primary {
                LengthPercentageOrAuto::LengthPercentage(lp) => resolve_lp(lp, basis),
                LengthPercentageOrAuto::Auto => match secondary {
                    LengthPercentageOrAuto::LengthPercentage(lp) => -resolve_lp(lp, basis),
                    LengthPercentageOrAuto::Auto => 0.0,
                },
            }
        };
    let dx = resolve(style.left, style.right, containing_width);
    let dy = resolve(style.top, style.bottom, 0.0);
    (dx, dy)
}

/// `style`/`border`(計算済みの太さ)の組み合わせが、実際に何か描画するか。
/// 背景色があるか、4辺のいずれかで太さが正かつ`border-style`が`none`でない
/// 場合に`true`(`pdf::document::render_box_decoration`が実際に描画する
/// 条件と同じ)。マイルストーン3の`Engine`が、`<body>`自身に装飾がないか
/// 判定する際にも使うため`pub(crate)`にしている。
pub(crate) fn has_visible_decoration(style: &ComputedStyle, border: &EdgeSizes) -> bool {
    if style.background_color.alpha > 0.0 {
        return true;
    }
    // `background-image`のみを持つ要素(背景色・枠線なし)も、`place_split`が
    // 装飾フラグメント(`node`付きの`LaidOutBox`)を生成する対象に含めない
    // 限り`collect_image_uses`/`render_box`から参照できず描画されない
    // ([0017](../../../docs/decisions/0017-background-image-design.md)決定2)。
    if style.background_image.is_some() {
        return true;
    }
    [
        (border.top, style.border_top_style),
        (border.right, style.border_right_style),
        (border.bottom, style.border_bottom_style),
        (border.left, style.border_left_style),
    ]
    .into_iter()
    .any(|(width, border_style)| width > 0.0 && border_style != BorderStyle::None)
}

pub(super) fn box_style(b: &LayoutBox, styles: &HashMap<NodeId, ComputedStyle>) -> ComputedStyle {
    match b.node {
        Some(node) => styles[&node].clone(),
        // 無名ボックス(CSS2.1 9.2.1.1)。マージン/パディング/枠線を持たないblock。
        None => ComputedStyle {
            display: Display::Block,
            ..ComputedStyle::default()
        },
    }
}

pub(super) fn resolve_lp(lp: LengthPercentage, basis: f32) -> f32 {
    match lp {
        LengthPercentage::Length(px) => px,
        LengthPercentage::Percentage(fraction) => fraction * basis,
    }
}

pub(crate) fn resolve_lpa_or_zero(lpa: LengthPercentageOrAuto, basis: f32) -> f32 {
    match lpa {
        LengthPercentageOrAuto::Auto => 0.0,
        LengthPercentageOrAuto::LengthPercentage(lp) => resolve_lp(lp, basis),
    }
}

pub(crate) fn resolve_padding(style: &ComputedStyle, basis: f32) -> EdgeSizes {
    EdgeSizes {
        top: resolve_lp(style.padding_top, basis),
        right: resolve_lp(style.padding_right, basis),
        bottom: resolve_lp(style.padding_bottom, basis),
        left: resolve_lp(style.padding_left, basis),
    }
}

/// `border-style: none`の辺は、`border-width`の指定に関わらず使用値が`0`になる
/// (CSS2.1 8.5.3)。レイアウト(幅計算)にもこの丸めが反映される必要がある。
pub(crate) fn resolve_border(style: &ComputedStyle) -> EdgeSizes {
    let width_or_zero = |width: Length, border_style: BorderStyle| {
        if border_style == BorderStyle::None {
            0.0
        } else {
            width.0
        }
    };
    EdgeSizes {
        top: width_or_zero(style.border_top_width, style.border_top_style),
        right: width_or_zero(style.border_right_width, style.border_right_style),
        bottom: width_or_zero(style.border_bottom_width, style.border_bottom_style),
        left: width_or_zero(style.border_left_width, style.border_left_style),
    }
}

/// `height`が明示指定されていれば返す。`auto`および(containing blockの高さが
/// 不定なため)パーセンテージ指定は`None`とし、呼び出し側でコンテンツ高さを使う。
/// `box-sizing: border-box`の場合、指定値は border-box の高さを表すため
/// `padding_tb`/`border_tb`を引いてcontent-box相当に変換する
/// ([0027](../../../docs/decisions/0027-box-sizing-design.md)決定2)。
fn resolve_height(style: &ComputedStyle, padding_tb: f32, border_tb: f32) -> Option<f32> {
    match style.height {
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => {
            Some(if style.box_sizing == BoxSizing::BorderBox {
                (px - padding_tb - border_tb).max(0.0)
            } else {
                px
            })
        }
        LengthPercentageOrAuto::Auto | LengthPercentageOrAuto::LengthPercentage(_) => None,
    }
}

/// 置換要素(`<img>`)のwidth/heightが両方`auto`の場合に限り、CSS2.2
/// §10.3.2/§10.6.2の簡略版(置換要素の内在サイズに基づく解決)を適用する:
/// HTML属性(`width`/`height`)→内在サイズ(デコード成功時)の優先順で決め、
/// 一方だけ値が得られる場合はアスペクト比を保って他方を導出する。
///
/// **既知の簡略化**: CSSで`width`/`height`のどちらか一方だけが明示指定
/// されている場合(もう一方は`auto`)は、通常のブロック要素と同じ扱いに
/// 委ねる(アスペクト比を保った導出は行わない)。実務上`<img>`にCSSで
/// 幅と高さを片方だけ指定するケースは稀であり、優先度を割かなかった。
fn apply_replaced_element_auto_size(style: &mut ComputedStyle, image: &ImageBoxContent) {
    let width_is_auto = matches!(style.width, LengthPercentageOrAuto::Auto);
    let height_is_auto = matches!(style.height, LengthPercentageOrAuto::Auto);
    if !(width_is_auto && height_is_auto) {
        return;
    }

    let attr_size = (
        image.attr_width.map(|w| w as f32),
        image.attr_height.map(|h| h as f32),
    );
    let intrinsic_size = image
        .image
        .as_ref()
        .map(|prepared| (prepared.width as f32, prepared.height as f32));

    let (width, height) = match attr_size {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (
            w,
            derive_via_aspect_ratio(w, intrinsic_size.map(|(iw, ih)| (ih, iw))),
        ),
        (None, Some(h)) => (derive_via_aspect_ratio(h, intrinsic_size), h),
        (None, None) => intrinsic_size.unwrap_or((0.0, 0.0)),
    };

    style.width = LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(width));
    style.height = LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(height));
}

/// `known`(既知の1辺の長さ)から、`ratio_basis`(`(既知でない辺の内在長,
/// 既知の辺の内在長)`)を使ってアスペクト比を保った他方の辺を導出する。
/// 内在サイズが無い(デコード失敗)、または既知の辺の内在長が0の場合は0を返す
/// (呼び出し側で「サイズ不明」の意味になる)。
fn derive_via_aspect_ratio(known: f32, ratio_basis: Option<(f32, f32)>) -> f32 {
    match ratio_basis {
        Some((other_intrinsic, known_intrinsic)) if known_intrinsic > 0.0 => {
            known * other_intrinsic / known_intrinsic
        }
        _ => 0.0,
    }
}

/// 2つの隣接するマージンを相殺(collapse)した結果の間隔を求める(CSS2.1 §8.3.1)。
/// 両方が非負なら大きい方、両方が負なら小さい方(絶対値が大きい方)、
/// 正負混在なら両者の単純な和(=正の最大値と負の最小値の和)になる。
fn collapse_adjacent_margins(a: f32, b: f32) -> f32 {
    let positive = a.max(0.0).max(b.max(0.0));
    let negative = a.min(0.0).min(b.min(0.0));
    positive + negative
}

/// CSS2.1 §10.3.3(block-level, non-replaced要素)の簡略版。
/// `margin-left + border-left + padding-left + width + padding-right + border-right + margin-right
/// = containing blockの幅`という制約から、`auto`な項目を埋める。
pub(crate) fn resolve_width_and_horizontal_margins(
    style: &ComputedStyle,
    containing_width: f32,
    padding_lr: f32,
    border_lr: f32,
) -> (f32, f32, f32) {
    let margin_left_is_auto = matches!(style.margin_left, LengthPercentageOrAuto::Auto);
    let margin_right_is_auto = matches!(style.margin_right, LengthPercentageOrAuto::Auto);

    if matches!(style.width, LengthPercentageOrAuto::Auto) {
        let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
        let margin_right = resolve_lpa_or_zero(style.margin_right, containing_width);
        let width =
            (containing_width - margin_left - border_lr - padding_lr - margin_right).max(0.0);
        return (width, margin_left, margin_right);
    }

    // `box-sizing: border-box`の場合、指定値はborder-boxの幅を表すため、
    // padding+borderを引いてcontent-box相当に変換してから既存の等式へ渡す
    // ([0027]決定2)。
    let width = resolve_lpa_or_zero(style.width, containing_width);
    let width = if style.box_sizing == BoxSizing::BorderBox {
        (width - padding_lr - border_lr).max(0.0)
    } else {
        width
    };
    let remaining = (containing_width - border_lr - padding_lr - width).max(0.0);

    match (margin_left_is_auto, margin_right_is_auto) {
        (true, true) => {
            let half = remaining / 2.0;
            (width, half, half)
        }
        (true, false) => {
            let margin_right = resolve_lpa_or_zero(style.margin_right, containing_width);
            (width, (remaining - margin_right).max(0.0), margin_right)
        }
        (false, true) => {
            let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
            (width, margin_left, (remaining - margin_left).max(0.0))
        }
        (false, false) => {
            // over-constrained(CSS2.1 §10.3.3): width/margin-left/margin-rightが
            // 全て明示指定されている場合、指定されたmargin-rightの値は無視し、
            // 等式(margin-left + border/padding + width + margin-right =
            // containing width)がちょうど成り立つよう使用値を再計算する
            // (負の値になることもある。`direction: rtl`時はmargin-left側を
            // 再計算すべきだが、rtl自体が未対応のため常にltr前提)。
            let margin_left = resolve_lpa_or_zero(style.margin_left, containing_width);
            let margin_right = containing_width - border_lr - padding_lr - width - margin_left;
            (width, margin_left, margin_right)
        }
    }
}

/// `b`の部分木全体のY座標を`delta`だけ平行移動した複製を返す。`paginate.rs`が
/// 1ページ全体の連続座標からページ内相対座標への変換に使う(`delta`を引く)。
/// `table.rs`がcaptionを`caption-side: bottom`で配置する際にも使う(`delta`に
/// 負の値を渡すことで下方向に移動する)。
pub(super) fn shift_box_y(b: &LaidOutBox, delta: f32) -> LaidOutBox {
    let mut b = b.clone();
    shift_rect_y(&mut b.layout.content, delta);
    if let Some(marker) = &mut b.marker {
        shift_rect_y(&mut marker.rect, delta);
    }

    match &mut b.content {
        LaidOutContent::Blocks(children) => {
            for child in children.iter_mut() {
                *child = shift_box_y(child, delta);
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines.iter_mut() {
                shift_rect_y(&mut line.rect, delta);
            }
        }
        LaidOutContent::Table(table) => {
            if let Some(caption) = &mut table.caption {
                **caption = shift_box_y(caption, delta);
            }
            for row in table.rows.iter_mut() {
                for cell in row.cells.iter_mut() {
                    *cell = shift_box_y(cell, delta);
                }
            }
        }
        // `b.layout.content`の平行移動(この関数冒頭)だけで十分。画像は
        // `Inline`の行のような、それ自身が別途Rectを持つ子要素を持たない。
        LaidOutContent::Image(_) => {}
    }

    b
}

fn shift_rect_y(rect: &mut Rect, delta: f32) {
    rect.y -= delta;
}

/// `b`自身の位置(`b.layout`)は変えず、その内容(子ボックス/行/テーブルの
/// 行・セル)だけを縦にシフトする。`shift_box_y`(自身含めた全体を平行移動)
/// とは別物として明確に区別する: テーブルセルの`vertical-align`実装では、
/// セル自身の高さ・位置は行の高さ均等化で既に確定済みで変えたくないが、
/// その内側の内容だけをtop/middle/bottom/baselineに応じて上下させたい
/// ([0021](../../../docs/decisions/0021-table-layout-design.md)決定4)。
pub(super) fn shift_content_vertical(b: &LaidOutBox, delta: f32) -> LaidOutBox {
    let mut b = b.clone();

    match &mut b.content {
        LaidOutContent::Blocks(children) => {
            for child in children.iter_mut() {
                *child = shift_box_y(child, delta);
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines.iter_mut() {
                shift_rect_y(&mut line.rect, delta);
            }
        }
        LaidOutContent::Table(table) => {
            if let Some(caption) = &mut table.caption {
                **caption = shift_box_y(caption, delta);
            }
            for row in table.rows.iter_mut() {
                for cell in row.cells.iter_mut() {
                    *cell = shift_box_y(cell, delta);
                }
            }
        }
        // `Image`は`Inline`の行のような、それ自身が別途Rectを持つ子要素を
        // 持たないため、動かす対象が無い(セルにネストした画像の
        // `vertical-align`は、セル内容全体を1つのブロックとして動かす形に
        // 委ねる)。
        LaidOutContent::Image(_) => {}
    }

    b
}

#[cfg(test)]
mod tests {
    use super::super::box_tree::build_box_tree;
    use super::*;
    use crate::fonts::Font;
    use crate::html::{self, Dom, NodeData};
    use crate::pdf::{ImagePlane, PlaneColorSpace};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(TEST_FONT_PATH).expect("should load bundled test font")
        ])
    }

    fn find_all(dom: &Dom, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                out.push(id);
            }
        }
        for child in dom.children(id) {
            find_all(dom, child, tag, out);
        }
    }

    fn find_box(b: &LayoutBox, target: NodeId) -> Option<&LayoutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        if let BoxContent::Blocks(children) = &b.content {
            for child in children {
                if let Some(found) = find_box(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        if let LaidOutContent::Blocks(children) = &b.content {
            for child in children {
                if let Some(found) = find_laid_out(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn display_none_excludes_element_and_subtree() {
        let dom = html::parse(
            br#"<div><p class="hidden">hidden</p><p class="visible">visible</p></div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".hidden { display: none; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let (hidden_p, visible_p) = (ps[0], ps[1]);

        assert!(find_box(&tree, hidden_p).is_none());
        assert!(find_box(&tree, visible_p).is_some());
    }

    #[test]
    fn mixed_block_and_inline_children_get_anonymous_block_wrapping() {
        let dom = html::parse(br#"<div class="outer">before <p>P</p> after</div>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);

        let div_box = find_box(&tree, divs[0]).expect("div box not found");
        let BoxContent::Blocks(children) = &div_box.content else {
            panic!("expected block container")
        };
        assert_eq!(children.len(), 3, "before-text / <p> / after-text");
        let joined_text = |content: &BoxContent| match content {
            BoxContent::Inline(spans) => spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            BoxContent::Blocks(_) | BoxContent::Table(_) | BoxContent::Image(_) => {
                panic!("expected inline content")
            }
        };
        assert_eq!(joined_text(&children[0].content).trim(), "before");
        assert_eq!(children[1].node, Some(ps[0]));
        assert_eq!(joined_text(&children[2].content).trim(), "after");
    }

    #[test]
    fn auto_width_fills_containing_block_minus_margins() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".box { margin: 10px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        // html: margin/padding/borderなし → content_width=800
        // body: UAデフォルトのmargin:8px → content_width=784
        // div: margin:10px → content_width=764
        assert_eq!(div_box.layout.margin.left, 10.0);
        assert_eq!(div_box.layout.content.width, 764.0);
        assert_eq!(div_box.layout.content.x, 18.0);
    }

    #[test]
    fn auto_margins_center_element_with_explicit_width() {
        let dom = html::parse(br#"<div class="centered"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".centered { width: 400px; margin: 0 auto; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 400.0);
        assert_eq!(div_box.layout.margin.left, div_box.layout.margin.right);
        assert_eq!(div_box.layout.margin.left, 192.0);
    }

    #[test]
    fn over_constrained_box_recalculates_margin_right_to_fit_the_containing_block() {
        // width/margin-left/margin-rightが全て明示指定され、かつ合計が
        // containing widthと一致しない(over-constrained)場合、CSS2.1 §10.3.3
        // に従い指定されたmargin-rightは無視され、等式が成り立つよう再計算される。
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        // containing width = 784(html:800, body margin:8pxずつ)。
        // width:300 + margin-left:50 + 指定margin-right:50 = 400 だが、
        // 784になるようmargin-rightは434に再計算されるはず。
        let author = parse_stylesheet(".box { width: 300px; margin: 0 50px 0 50px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 300.0);
        assert_eq!(div_box.layout.margin.left, 50.0);
        assert_eq!(
            div_box.layout.margin.right, 434.0,
            "over-constrained margin-right should be recalculated, not the specified 50px"
        );
    }

    #[test]
    fn over_constrained_recalculation_can_produce_a_negative_margin_right() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        // containing width = 784。width自体がそれを埋め尽くすので、margin-leftの
        // 分だけ超過し、再計算後のmargin-rightは指定値(99px)と符号すら異なる
        // 負の値になるはず。
        let author = parse_stylesheet(".box { width: 784px; margin: 0 99px 0 30px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.margin.right, -30.0);
    }

    #[test]
    fn block_siblings_stack_vertically_by_content_height() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet(".a { height: 50px; margin: 0; } .b { height: 30px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        assert_eq!(
            b.layout.content.y,
            a.layout.content.y + a.layout.content.height
        );
    }

    #[test]
    fn equal_adjacent_margins_collapse_to_a_single_gap_instead_of_summing() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        // 両方とも上下16pxのマージン。相殺されていれば、border-box間の隙間は
        // 32px(単純な加算)ではなく16pxになるはず。
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 16px 0; } .b { height: 20px; margin: 16px 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 16.0,
            "equal adjacent margins should collapse to their shared value"
        );
    }

    #[test]
    fn left_float_is_removed_from_normal_flow_and_placed_at_containing_left() {
        let dom = html::parse(
            br#"<div class="outer"><div class="f">F</div><div class="after">after</div></div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } \
             .f { float: left; width: 100px; height: 50px; } \
             .after { height: 20px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let float_box = find_laid_out(&laid, divs[1]).expect("float box not found");
        let after_box = find_laid_out(&laid, divs[2]).expect("after box not found");

        assert!(float_box.is_float);
        assert_eq!(float_box.layout.content.x, 0.0);
        assert_eq!(float_box.layout.content.y, 0.0);
        // floatはフローに参加しないため、後続のブロックはfloatの高さ(50px)を
        // 無視してcontaining blockの先頭からすぐ配置される([0019]決定5前提)。
        assert_eq!(after_box.layout.content.y, 0.0);
    }

    #[test]
    fn right_float_is_placed_against_the_containing_right_edge() {
        let dom = html::parse(br#"<div class="outer"><div class="f">F</div></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } .f { float: right; width: 100px; height: 50px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let float_box = find_laid_out(&laid, divs[1]).expect("float box not found");

        assert_eq!(float_box.layout.content.x, 700.0);
        assert_eq!(float_box.layout.content.y, 0.0);
    }

    #[test]
    fn second_left_float_packs_next_to_the_first_instead_of_stacking() {
        let dom = html::parse(
            br#"<div class="outer"><div class="a">A</div><div class="b">B</div></div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } \
             .a { float: left; width: 100px; height: 50px; } \
             .b { float: left; width: 100px; height: 30px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let a_box = find_laid_out(&laid, divs[1]).expect("a not found");
        let b_box = find_laid_out(&laid, divs[2]).expect("b not found");

        assert_eq!(a_box.layout.content.x, 0.0);
        assert_eq!(b_box.layout.content.x, 100.0);
        assert_eq!(b_box.layout.content.y, 0.0);
    }

    #[test]
    fn clear_pushes_the_element_below_the_float() {
        let dom = html::parse(
            br#"<div class="outer"><div class="f">F</div><div class="c">after</div></div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } \
             .f { float: left; width: 100px; height: 50px; } \
             .c { clear: left; height: 20px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let cleared_box = find_laid_out(&laid, divs[2]).expect("cleared box not found");

        assert_eq!(cleared_box.layout.content.y, 50.0);
    }

    #[test]
    fn float_does_not_participate_in_adjacent_margin_collapsing() {
        let dom = html::parse(
            br#"<div class="outer">
                <div class="a">a</div><div class="f">F</div><div class="b">b</div>
                </div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } \
             .a { height: 10px; margin: 0 0 20px 0; } \
             .f { float: left; width: 30px; height: 5px; } \
             .b { height: 10px; margin: 30px 0 0 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let a_box = find_laid_out(&laid, divs[1]).expect("a not found");
        let b_box = find_laid_out(&laid, divs[3]).expect("b not found");

        assert_eq!(a_box.layout.content.y, 0.0);
        // aとbの間にfloatを挟んでいても、直前の非float子(a)とのマージン相殺が
        // そのまま働く: max(20, 30) = 30。floatをマージン相殺の対象に含めて
        // しまうと(floatはmarginを持たないため0とみなされ)この値がずれる。
        assert_eq!(b_box.layout.content.y, 40.0);
    }

    #[test]
    fn container_auto_height_expands_to_include_a_taller_float_child() {
        let dom = html::parse(br#"<div class="outer"><div class="f">F</div></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet("body { margin: 0; } .f { float: left; width: 50px; height: 200px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let outer_box = find_laid_out(&laid, divs[0]).expect("outer not found");

        assert_eq!(outer_box.layout.content.height, 200.0);
    }

    #[test]
    fn position_relative_offsets_visual_position_without_affecting_siblings() {
        let dom = html::parse(
            br#"<div class="outer">
                <div class="a">a</div><div class="rel">b</div><div class="c">c</div>
                </div>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            "body { margin: 0; } \
             .a { height: 10px; } \
             .rel { position: relative; top: 5px; left: 7px; height: 20px; } \
             .c { height: 10px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let rel_box = find_laid_out(&laid, divs[2]).expect("rel not found");
        let c_box = find_laid_out(&laid, divs[3]).expect("c not found");

        // 通常位置はx=0, y=10(aの下)だが、top:5px/left:7pxのオフセットが加わる。
        assert_eq!(rel_box.layout.content.x, 7.0);
        assert_eq!(rel_box.layout.content.y, 15.0);
        // cはrel要素本来の(オフセット前の)下端(10+20=30)を基準に配置され、
        // 視覚的オフセットの影響を受けない([0019]決定6)。
        assert_eq!(c_box.layout.content.y, 30.0);
    }

    #[test]
    fn unequal_adjacent_margins_collapse_to_the_larger_one() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 0 0 10px 0; } .b { height: 20px; margin: 24px 0 0 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 24.0,
            "collapsed gap should be the larger of the two margins"
        );
    }

    #[test]
    fn a_negative_margin_reduces_the_collapsed_gap() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 20px; margin: 0 0 10px 0; } .b { height: 20px; margin: -4px 0 0 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let a = find_laid_out(&laid, ps[0]).expect("p.a not found");
        let b = find_laid_out(&laid, ps[1]).expect("p.b not found");

        let gap =
            b.layout.border_box().y - (a.layout.border_box().y + a.layout.border_box().height);
        assert_eq!(
            gap, 6.0,
            "positive + negative margins should sum (10 + (-4) = 6)"
        );
    }

    #[test]
    fn parent_and_first_child_margins_are_not_collapsed() {
        // 親子間のマージン相殺は本実装のスコープ外(隣接兄弟間のみ対応)。
        // 最初の子の上マージンは、親のcontent開始位置にそのまま加算されるはず。
        let dom = html::parse(br#"<div class="outer"><p class="inner">x</p></div>"#);
        let ua = user_agent_stylesheet();
        let author =
            parse_stylesheet(".outer { margin: 0; } .inner { height: 20px; margin: 12px 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let p = find_laid_out(&laid, ps[0]).expect("p not found");

        assert_eq!(
            p.layout.margin.top, 12.0,
            "the child's own top margin should still apply in full (no parent-child collapsing)"
        );
    }

    #[test]
    fn auto_height_block_sizes_to_children_content() {
        let dom = html::parse(br#"<div class="outer"><p class="inner">x</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".inner { height: 40px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let outer = find_laid_out(&laid, divs[0]).expect("outer div not found");

        assert_eq!(outer.layout.content.height, 40.0);
    }

    #[test]
    fn wrapped_inline_content_drives_auto_height() {
        // 十分な幅があれば1行、狭ければ複数行に折り返される。
        let dom = html::parse(br#"<p class="a">hello world</p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);

        let wide = layout_document(&tree, &styles, &fonts, 800.0);
        let p_wide = find_laid_out(&wide, ps[0]).expect("p not found");
        let LaidOutContent::Inline(lines_wide) = &p_wide.content else {
            panic!("expected inline content")
        };
        assert_eq!(lines_wide.len(), 1);

        let narrow = layout_document(&tree, &styles, &fonts, 60.0);
        let p_narrow = find_laid_out(&narrow, ps[0]).expect("p not found");
        let LaidOutContent::Inline(lines_narrow) = &p_narrow.content else {
            panic!("expected inline content")
        };
        assert_eq!(lines_narrow.len(), 2);

        assert!(p_narrow.layout.content.height > p_wide.layout.content.height);
    }

    #[test]
    fn padding_and_border_offset_content_box() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { width: 100px; margin: 0; padding: 5px; border: 2px solid black; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 100.0);
        assert_eq!(div_box.layout.padding.left, 5.0);
        assert_eq!(div_box.layout.border.left, 2.0);

        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 2.0 + 5.0 + 100.0 + 5.0 + 2.0);
    }

    #[test]
    fn box_sizing_border_box_makes_the_specified_width_include_padding_and_border() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { box-sizing: border-box; width: 100px; height: 60px; margin: 0; \
             padding: 5px; border: 2px solid black; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        // border-boxでは指定した100px/60pxがpadding+border込みの外寸になるため、
        // content-boxはその分小さくなる(100 - 2*5 - 2*2 = 86)。
        assert_eq!(div_box.layout.content.width, 100.0 - 2.0 * 5.0 - 2.0 * 2.0);
        assert_eq!(div_box.layout.content.height, 60.0 - 2.0 * 5.0 - 2.0 * 2.0);

        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 100.0);
        assert_eq!(border_box.height, 60.0);
    }

    #[test]
    fn box_sizing_border_box_clamps_to_zero_when_padding_and_border_exceed_the_specified_width() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { box-sizing: border-box; width: 5px; margin: 0; \
             padding: 10px; border: 10px solid black; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.content.width, 0.0);
    }

    #[test]
    fn border_style_none_zeroes_out_the_used_border_width_in_layout() {
        // CSS2.1 8.5.3: border-styleがnoneの辺は、border-widthの指定に関わらず
        // 使用値が0になる(枠線が描画されないだけでなく、レイアウト上の
        // 幅計算にも影響しない)。
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { width: 100px; margin: 0; border-width: 5px; border-style: none; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(div_box.layout.border.left, 0.0);
        let border_box = div_box.layout.border_box();
        assert_eq!(border_box.width, 100.0);
    }

    #[test]
    fn fragmentation_hints_reflect_the_elements_computed_style() {
        let dom = html::parse(br#"<div class="box"></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".box { break-before: always; break-inside: avoid; orphans: 3; widows: 4; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");

        assert_eq!(
            div_box.fragmentation.break_before,
            super::BreakBetween::Always
        );
        assert_eq!(div_box.fragmentation.break_after, super::BreakBetween::Auto);
        assert_eq!(
            div_box.fragmentation.break_inside,
            super::BreakInside::Avoid
        );
        assert_eq!(div_box.fragmentation.orphans, 3);
        assert_eq!(div_box.fragmentation.widows, 4);
    }

    #[test]
    fn anonymous_boxes_get_default_fragmentation_hints() {
        // 無名ボックス(混在コンテンツの折り返し等)は対応するDOM要素を持たないため、
        // fragmentationヒントは常に初期値(auto/auto/auto/2/2)になるはず。
        let dom = html::parse(br#"<div class="outer">before <p>P</p> after</div>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let div_box = find_laid_out(&laid, divs[0]).expect("div box not found");
        let LaidOutContent::Blocks(children) = &div_box.content else {
            panic!("expected block container")
        };
        let anonymous = children
            .iter()
            .find(|c| c.node.is_none())
            .expect("expected an anonymous block wrapping the loose text");

        assert_eq!(anonymous.fragmentation, FragmentationHints::default());
    }

    fn image_prepared(width: u32, height: u32) -> Rc<PreparedImage> {
        Rc::new(PreparedImage {
            width,
            height,
            color: ImagePlane {
                data: Vec::new(),
                filter: pdf_writer::Filter::FlateDecode,
                color_space: PlaneColorSpace::Rgb,
                bits_per_component: 8,
            },
            alpha: None,
        })
    }

    fn image_box(content: ImageBoxContent) -> LayoutBox {
        LayoutBox {
            node: None,
            content: BoxContent::Image(content),
            marker: None,
        }
    }

    #[test]
    fn image_with_no_attrs_uses_intrinsic_size_when_decoded() {
        let tree = image_box(ImageBoxContent {
            image: Some(image_prepared(200, 100)),
            attr_width: None,
            attr_height: None,
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 200.0);
        assert_eq!(laid.layout.content.height, 100.0);
    }

    #[test]
    fn image_width_attr_only_derives_height_via_aspect_ratio() {
        // 内在サイズは200x100(2:1)。width=50pxのみ指定 → height=25px。
        let tree = image_box(ImageBoxContent {
            image: Some(image_prepared(200, 100)),
            attr_width: Some(50),
            attr_height: None,
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 50.0);
        assert_eq!(laid.layout.content.height, 25.0);
    }

    #[test]
    fn image_height_attr_only_derives_width_via_aspect_ratio() {
        let tree = image_box(ImageBoxContent {
            image: Some(image_prepared(200, 100)),
            attr_width: None,
            attr_height: Some(40),
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.height, 40.0);
        assert_eq!(laid.layout.content.width, 80.0);
    }

    #[test]
    fn image_with_both_attrs_ignores_the_intrinsic_aspect_ratio() {
        let tree = image_box(ImageBoxContent {
            image: Some(image_prepared(200, 100)),
            attr_width: Some(10),
            attr_height: Some(10),
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 10.0);
        assert_eq!(laid.layout.content.height, 10.0);
    }

    #[test]
    fn failed_image_with_no_attrs_collapses_to_zero_size() {
        let tree = image_box(ImageBoxContent {
            image: None,
            attr_width: None,
            attr_height: None,
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 0.0);
        assert_eq!(laid.layout.content.height, 0.0);
    }

    #[test]
    fn failed_image_with_explicit_attrs_still_reserves_the_specified_space() {
        // [0014]の方針: 取得失敗でもwidth/height属性があればそのサイズの
        // 空ボックスとして扱う(後続コンテンツが不意にレイアウトが
        // 詰まらないよう、指定サイズ分のスペースは確保する)。
        let tree = image_box(ImageBoxContent {
            image: None,
            attr_width: Some(50),
            attr_height: Some(50),
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 50.0);
        assert_eq!(laid.layout.content.height, 50.0);
    }

    #[test]
    fn image_does_not_stretch_to_fill_the_containing_block_like_a_block_div_would() {
        // 通常のブロック要素はwidth:autoでcontaining blockいっぱいに広がるが、
        // 置換要素はそうならない(内在サイズをそのまま使う)ことの確認。
        let tree = image_box(ImageBoxContent {
            image: Some(image_prepared(50, 50)),
            attr_width: None,
            attr_height: None,
        });
        let laid = layout_document(&tree, &HashMap::new(), &test_fonts(), 800.0);

        assert_eq!(laid.layout.content.width, 50.0);
    }

    #[test]
    fn outside_marker_is_positioned_left_of_the_content_edge_with_a_fixed_gap() {
        let dom = html::parse(br#"<ul><li>text</li></ul>"#);
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let mut lis = Vec::new();
        find_all(&dom, dom.document(), "li", &mut lis);
        let li = find_laid_out(&laid, lis[0]).expect("li not found");

        let marker = li.marker.as_ref().expect("li should have a marker");
        assert_eq!(marker.runs.len(), 1);
        assert!(marker.rect.width > 0.0);
        assert_eq!(
            marker.rect.x,
            li.layout.content.x - LIST_MARKER_GAP - marker.rect.width
        );
        assert_eq!(
            marker.rect.y, li.layout.content.y,
            "marker should align with the top of the li's own content"
        );
    }

    #[test]
    fn list_style_type_none_produces_no_marker_in_the_laid_out_box() {
        let dom = html::parse(br#"<ul><li style="list-style-type: none;">text</li></ul>"#);
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, 800.0);

        let li = find(&dom, dom.document(), "li").expect("li not found");
        let li_laid = find_laid_out(&laid, li).expect("li not found");
        assert!(li_laid.marker.is_none());
    }

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }
}
