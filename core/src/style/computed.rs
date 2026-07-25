//! カスケード済み宣言(T3)から、要素ごとの計算スタイルを算出する。
//!
//! プロパティごとに「宣言があればそれを採用(カスケード順で最後に勝ったもの)、
//! なければ継承プロパティは親から継承、そうでなければ初期値」という
//! CSSの計算値算出手順を実装する。

use std::cell::Cell;
use std::collections::HashMap;

use crate::html::{Dom, NodeData, NodeId};

use super::cascade::{
    matching_declarations_by_origin, matching_pseudo_content, matching_pseudo_declarations,
};
use super::presentational::presentational_hint_declarations;
use super::properties::PropertyDeclaration;
use super::selector_impl::PseudoElement;
use super::stylesheet::{parse_inline_style, Stylesheet};
use super::values::{
    AlignContent, AlignItems, AlignSelf, AspectRatio, BackgroundAttachment, BackgroundPosition,
    BackgroundRepeat, BackgroundSize, BorderCollapse, BorderStyle, BoxSizing, BreakBetween,
    BreakInside, CaptionSide, Clear, Color, ContentPart, CornerRadius, Display, EmphasisPosition,
    EmphasisStyle, EmptyCells, FlexBasis, FlexDirection, FlexWrap, Float, FontStyle, FontWeight,
    GridArea, GridAutoFlow, GridLine, Hyphens, JustifyContent, Length, LengthPercentage,
    LengthPercentageOrAuto, ListStylePosition, ListStyleType, MaxSize, ObjectFit, Overflow,
    OverflowWrap, Position, QuotePair, SpecifiedCornerRadius, SpecifiedLength,
    SpecifiedLengthPercentage, SpecifiedLengthPercentageOrAuto, SpecifiedLineHeight,
    SpecifiedMaxSize, SpecifiedTrackSize, TableLayout, TextAlign, TextDecorationLine, TextOverflow,
    TextTransform, TrackList, TrackSize, TransformFunction, VerticalAlign, Visibility, WhiteSpace,
    WordBreak, ZIndex,
};

/// `color`/`background-color`の計算値。パース時と異なり`currentcolor`は解決済み。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbaColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: f32,
}

impl RgbaColor {
    /// 完全に透明(`background-color`の初期値`transparent`)。
    pub const TRANSPARENT: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 0.0,
    };
}

/// `line-height`の計算値。CSS2.1 §10.8.1: `<number>`/`<percentage>`の計算値は
/// 「指定値の数値そのもの」(親のfont-sizeで先に乗算した絶対値ではない)。
/// 継承時はこの値のまま伝わり、使用側(`layout::inline`)がそのテキストランの
/// font-sizeで乗算する([0020](
/// ../../../docs/decisions/0020-typography-details-design.md)決定3)。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LineHeight {
    #[default]
    Normal,
    /// `<number>`。`<percentage>`は`p/100.0`に正規化した上でこの値に入れる。
    Number(f32),
    /// `<length>`。em/rem解決済みの絶対px、そのまま継承される。
    Length(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub width: LengthPercentageOrAuto,
    pub height: LengthPercentageOrAuto,
    /// `min-width`/`min-height`。非継承プロパティ、初期値`0`([0051](
    /// ../../../docs/decisions/0051-min-max-size-design.md)決定1)。
    pub min_width: LengthPercentage,
    pub min_height: LengthPercentage,
    /// `max-width`/`max-height`。非継承プロパティ、初期値`none`(上限なし)。
    pub max_width: MaxSize,
    pub max_height: MaxSize,
    /// `aspect-ratio`。非継承プロパティ、初期値`auto`([0052](
    /// ../../../docs/decisions/0052-aspect-ratio-design.md))。置換要素(`<img>`)では
    /// 寸法解決の入口で内在比が`ratio`へ焼き込まれる(決定2)。
    pub aspect_ratio: AspectRatio,
    pub margin_top: LengthPercentageOrAuto,
    pub margin_right: LengthPercentageOrAuto,
    pub margin_bottom: LengthPercentageOrAuto,
    pub margin_left: LengthPercentageOrAuto,
    pub padding_top: LengthPercentage,
    pub padding_right: LengthPercentage,
    pub padding_bottom: LengthPercentage,
    pub padding_left: LengthPercentage,
    pub border_top_width: Length,
    pub border_right_width: Length,
    pub border_bottom_width: Length,
    pub border_left_width: Length,
    /// 初期値は`currentcolor`(仕様通り)。宣言がなければこの要素自身の
    /// 計算済み`color`を使う(`resolve_color`で解決)。
    pub border_top_color: RgbaColor,
    pub border_right_color: RgbaColor,
    pub border_bottom_color: RgbaColor,
    pub border_left_color: RgbaColor,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    /// 水平/垂直の半径を持つ([0023](../../../docs/decisions/0023-box-model-details-design.md)
    /// 決定6、真円は水平=垂直)。
    pub border_top_left_radius: CornerRadius,
    pub border_top_right_radius: CornerRadius,
    pub border_bottom_right_radius: CornerRadius,
    pub border_bottom_left_radius: CornerRadius,
    /// 継承プロパティ。
    pub font_size: Length,
    /// 継承プロパティ。
    pub font_family: Vec<String>,
    /// 継承プロパティ。
    pub font_weight: FontWeight,
    /// 継承プロパティ。
    pub font_style: FontStyle,
    /// 継承プロパティ。
    pub color: RgbaColor,
    pub background_color: RgbaColor,
    /// `url(...)`(生の値、解決は呼び出し側任せ)。非継承プロパティ、初期値`None`。
    /// [0017](../../../docs/decisions/0017-background-image-design.md)参照。
    pub background_image: Option<String>,
    /// 非継承プロパティ。[0025](../../../docs/decisions/0025-background-details-design.md)。
    pub background_position: BackgroundPosition,
    /// 非継承プロパティ。
    pub background_size: BackgroundSize,
    /// 非継承プロパティ。
    pub background_repeat: BackgroundRepeat,
    /// 非継承プロパティ。`fixed`は`scroll`と同一視して描画する(決定5)。
    pub background_attachment: BackgroundAttachment,
    /// `text-decoration-line`。仕様上は非継承プロパティだが、代わりに祖先の
    /// 装飾線が子孫のボックスへ「伝播」する特殊規則を持つ。この伝播を
    /// 別途実装する代わりに、継承プロパティとして扱うことで
    /// (`<u>bold <b>text</b></u>`のような)一般的なネストケースで見た目を一致させる
    /// 簡略実装。子孫側で明示的に上書きされれば通常の継承同様そちらが勝つ。
    pub text_decoration_line: TextDecorationLine,
    /// `::before { content: "..." }`の生成コンテンツ。この要素自身のスタイルを
    /// そのまま流用して描画する(擬似要素専用の計算スタイルは持たない簡略実装)。
    pub pseudo_before_content: Option<String>,
    /// `::after { content: "..." }`の生成コンテンツ。
    pub pseudo_after_content: Option<String>,
    /// CSS Fragmentation。非継承プロパティ(仕様通り)。
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    pub break_inside: BreakInside,
    /// ページ末尾に残せる最小行数。非継承プロパティ、初期値2(仕様通り)。
    pub orphans: u32,
    /// ページ先頭に送れる最小行数。非継承プロパティ、初期値2(仕様通り)。
    pub widows: u32,
    /// `float`。非継承プロパティ。`none`以外なら`display`はblock-levelとして
    /// 計算される(CSS2.1 9.7、下記`compute_element_style`で適用)。
    pub float: Float,
    /// `clear`。非継承プロパティ。
    pub clear: Clear,
    /// `position`。非継承プロパティ([0049])。
    /// ([0018](../../../docs/decisions/0018-css21-css3-coverage-strategy.md))。
    pub position: Position,
    pub top: LengthPercentageOrAuto,
    pub right: LengthPercentageOrAuto,
    pub bottom: LengthPercentageOrAuto,
    pub left: LengthPercentageOrAuto,
    /// 継承プロパティ。IFC内では先頭`InlineSpan`の計算値で代表する
    /// ([0020](../../../docs/decisions/0020-typography-details-design.md)決定4)。
    pub text_align: TextAlign,
    /// 継承プロパティ。`Number`/`Percentage`は未乗算のまま継承し、使用側
    /// (`layout::inline`)がテキストランのfont-sizeで乗算する(決定3)。
    pub line_height: LineHeight,
    /// 継承プロパティ。パーセンテージはcontaining block幅が未解決のため
    /// fractionのまま保持する(`width`/`margin`と同じ「使用値は使う側で解決」
    /// パターン)。IFC内では先頭`InlineSpan`の計算値で代表する(決定4)。
    pub text_indent: LengthPercentage,
    /// 継承プロパティ。IFC内では先頭`InlineSpan`の計算値で代表する(決定4)。
    pub white_space: WhiteSpace,
    /// 継承プロパティ。解決済みpx、`normal`は`0.0`。
    pub letter_spacing: f32,
    /// 継承プロパティ。解決済みpx、`normal`は`0.0`。
    pub word_spacing: f32,
    /// 継承プロパティ。
    pub text_transform: TextTransform,
    /// `text-shadow`。継承プロパティ、空のVecは`none`([0053](
    /// ../../../docs/decisions/0053-text-details-design.md)決定1)。色は解決済み。
    pub text_shadow: Vec<ComputedTextShadow>,
    /// `text-overflow`。**非**継承プロパティ(仕様通り)。`overflow`が
    /// `visible`以外のときにのみ効く(決定4)。
    pub text_overflow: TextOverflow,
    /// `word-break`。継承プロパティ(決定3)。
    pub word_break: WordBreak,
    /// `overflow-wrap`(別名`word-wrap`)。継承プロパティ(決定3)。
    pub overflow_wrap: OverflowWrap,
    /// `hyphens`。継承プロパティ(決定2)。
    pub hyphens: Hyphens,
    /// `text-emphasis-style`。継承プロパティ(決定6)。
    pub text_emphasis_style: EmphasisStyle,
    /// `text-emphasis-color`。継承プロパティ、初期値は`currentcolor`。
    pub text_emphasis_color: RgbaColor,
    /// `text-emphasis-position`。継承プロパティ、初期値`over`。
    pub text_emphasis_position: EmphasisPosition,
    /// `grid-template-columns`/`grid-template-rows`。非継承プロパティ、
    /// 空なら`none`([0054](../../../docs/decisions/0054-grid-design.md)決定3)。
    pub grid_template_columns: TrackList,
    pub grid_template_rows: TrackList,
    /// `grid-auto-columns`/`grid-auto-rows`。非継承、空なら初期値`auto`。
    pub grid_auto_columns: Vec<TrackSize>,
    pub grid_auto_rows: Vec<TrackSize>,
    /// `grid-auto-flow`。非継承、初期値`row`。
    pub grid_auto_flow: GridAutoFlow,
    /// `grid-template-areas`。非継承、空なら`none`(決定4)。
    pub grid_template_areas: Vec<GridArea>,
    /// `grid-row-start`等。非継承、初期値`auto`(決定5)。
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    /// `justify-items`/`justify-self`。非継承。**Gridでのみ意味を持つ**
    /// (flexアイテムには適用されない、決定7)。
    pub justify_items: AlignItems,
    pub justify_self: AlignSelf,
    /// `border-collapse`。継承プロパティ。見た目の枠線描画のみ統合する
    /// ([0021](../../../docs/decisions/0021-table-layout-design.md)決定1)。
    pub border_collapse: BorderCollapse,
    /// `border-spacing`の水平方向。継承プロパティ、`border-collapse: collapse`
    /// 時は無視され0として扱う(仕様通り、`layout::table`側で解決)。
    pub border_spacing_horizontal: Length,
    /// `border-spacing`の垂直方向。継承プロパティ。
    pub border_spacing_vertical: Length,
    /// `caption-side`。継承プロパティ。
    pub caption_side: CaptionSide,
    /// `table-layout`。非継承プロパティ、テーブル要素自身の値を使う。
    pub table_layout: TableLayout,
    /// `empty-cells`。継承プロパティ、`border-collapse: separate`でのみ意味を持つ。
    pub empty_cells: EmptyCells,
    /// `vertical-align`(テーブルセル文脈専用)。非継承プロパティ。
    pub vertical_align: VerticalAlign,
    /// `list-style-type`。継承プロパティ([0022](
    /// ../../../docs/decisions/0022-list-style-design.md)決定2)。
    pub list_style_type: ListStyleType,
    /// `list-style-position`。継承プロパティ(決定4)。
    pub list_style_position: ListStylePosition,
    /// `list-style-image`(`url(...)`の生の値)。継承プロパティだが、実際には
    /// 常に`list_style_type`のテキストマーカーへフォールバックし描画されない
    /// (決定5)。
    pub list_style_image: Option<String>,
    /// `overflow`。非継承プロパティ。`hidden`/`scroll`/`auto`は区別せず全て
    /// クリップ対象として扱う([0023](../../../docs/decisions/0023-box-model-details-design.md)
    /// 決定1)。
    pub overflow: Overflow,
    /// `box-sizing`。非継承プロパティ。[0027](../../../docs/decisions/0027-box-sizing-design.md)。
    pub box_sizing: BoxSizing,
    /// `z-index`。非継承プロパティ。`position: static`の要素には効果を持たない
    /// (仕様通り、`layout`/`pdf`側で判定する、決定2)。
    pub z_index: ZIndex,
    /// `visibility`。継承プロパティ。`collapse`は`hidden`と同一視する(決定4)。
    pub visibility: Visibility,
    /// `outline-width`。非継承プロパティ。
    pub outline_width: Length,
    /// `outline-style`。非継承プロパティ、初期値`none`。
    pub outline_style: BorderStyle,
    /// `outline-color`。非継承プロパティ、初期値は`currentcolor`相当
    /// (`border-color`と同じ解決規則)。
    pub outline_color: RgbaColor,
    /// `quotes`。継承プロパティ。`None`は`none`(常に空文字列を生成する、
    /// [0024](../../../docs/decisions/0024-generated-content-design.md)決定3)。
    pub quotes: Option<Vec<QuotePair>>,
    /// `::first-letter`の限定的な上書きスタイル(決定4)。マッチする宣言が
    /// 一つも無ければ`None`。
    pub first_letter_style: Option<FirstLetterStyle>,
    /// `object-fit`。非継承プロパティ、`<img>`にのみ意味を持つ。
    /// [0030](../../../docs/decisions/0030-object-fit-position-design.md)。
    pub object_fit: ObjectFit,
    /// `object-position`。非継承プロパティ、初期値`50% 50%`
    /// (`background-position`の初期値`0% 0%`とは異なる)。
    pub object_position: BackgroundPosition,
    /// `box-shadow`。非継承プロパティ、初期値は空(影なし)。カンマ区切りの
    /// 複数指定に対応、先頭が最前面(決定2)。[0032](
    /// ../../../docs/decisions/0032-box-shadow-design.md)。
    pub box_shadow: Vec<ComputedBoxShadow>,
    /// `flex-direction`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    /// [0034](../../../docs/decisions/0034-flexbox-design.md)。
    pub flex_direction: FlexDirection,
    /// `flex-wrap`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub flex_wrap: FlexWrap,
    /// `justify-content`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub justify_content: JustifyContent,
    /// `align-items`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub align_items: AlignItems,
    /// `align-content`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub align_content: AlignContent,
    /// `align-self`。非継承プロパティ、flexアイテムにのみ意味を持つ。
    pub align_self: AlignSelf,
    /// `flex-grow`。非継承プロパティ、flexアイテムにのみ意味を持つ。
    pub flex_grow: f32,
    /// `flex-shrink`。非継承プロパティ、flexアイテムにのみ意味を持つ。
    pub flex_shrink: f32,
    /// `flex-basis`。非継承プロパティ、flexアイテムにのみ意味を持つ。
    pub flex_basis: FlexBasis,
    /// `row-gap`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub row_gap: LengthPercentage,
    /// `column-gap`。非継承プロパティ、flexコンテナ自身にのみ意味を持つ。
    pub column_gap: LengthPercentage,
    /// `transform`。非継承プロパティ。パーセンテージ(`translate`系)は要素
    /// 自身のborder-boxサイズが確定してから解決するため未解決のまま保持する
    /// ([0035](../../../docs/decisions/0035-opacity-transform-design.md)決定1-2)。
    /// 空のVecは`none`。
    pub transform: Vec<TransformFunction>,
    /// `transform-origin`。非継承プロパティ、初期値`50% 50%`
    /// (`background-position`の初期値`0% 0%`とは異なる、決定1-1)。
    pub transform_origin: BackgroundPosition,
    /// `opacity`。非継承プロパティ、0〜1にクランプ済み、初期値1.0
    /// ([0035]決定2)。
    pub opacity: f32,
}

/// `box-shadow`1つ分の計算値。長さはpx解決済み、`color`は`currentcolor`を
/// 解決済み(`resolve_color`、この要素自身の計算済み`color`を基準にする)。
/// [0032](../../../docs/decisions/0032-box-shadow-design.md)参照。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedBoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: RgbaColor,
    /// `inset`キーワード。パースはするが描画は非対応(決定1、既知の簡略化)。
    pub inset: bool,
}

/// `grid-auto-columns`/`grid-auto-rows`の`em`/`rem`解決。
fn resolve_track_sizes(
    sizes: &[SpecifiedTrackSize],
    font_size: f32,
    root_font_size: f32,
) -> Vec<TrackSize> {
    sizes
        .iter()
        .map(|size| size.resolve(font_size, root_font_size))
        .collect()
}

/// `text-shadow`1つ分の計算値。長さはpx解決済み、`color`は`currentcolor`を
/// 解決済み([0053](../../../docs/decisions/0053-text-details-design.md)決定5)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedTextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: RgbaColor,
}

/// `::first-letter`用の限定的な上書きスタイル。実装コストと需要のバランスを
/// 取り、フォント系・color・text-decoration-line・text-transformのみ対応する
/// ([0024](../../../docs/decisions/0024-generated-content-design.md)決定4、
/// `float`/box model系プロパティは非対応)。各フィールドが`None`の場合は
/// ホスト要素自身の計算値をそのまま使う。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FirstLetterStyle {
    pub font_size: Option<Length>,
    pub font_family: Option<Vec<String>>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub color: Option<RgbaColor>,
    pub text_decoration_line: Option<TextDecorationLine>,
    pub text_transform: Option<TextTransform>,
}

impl Default for ComputedStyle {
    /// CSSの初期値。`border-width`の初期値は仕様上`medium`(実装依存の太さ、
    /// 概ね3px相当)だが、意図しない既定枠線の描画を避けるためここでは`0`とする
    /// (どのみち`border-style`の初期値`none`により、幅があっても描画はされない)。
    fn default() -> Self {
        let zero_lp = LengthPercentage::Length(0.0);
        Self {
            display: Display::Inline,
            width: LengthPercentageOrAuto::Auto,
            height: LengthPercentageOrAuto::Auto,
            min_width: zero_lp,
            min_height: zero_lp,
            max_width: MaxSize::None,
            max_height: MaxSize::None,
            aspect_ratio: AspectRatio::default(),
            margin_top: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_right: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_bottom: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            margin_left: LengthPercentageOrAuto::LengthPercentage(zero_lp),
            padding_top: zero_lp,
            padding_right: zero_lp,
            padding_bottom: zero_lp,
            padding_left: zero_lp,
            border_top_width: Length(0.0),
            border_right_width: Length(0.0),
            border_bottom_width: Length(0.0),
            border_left_width: Length(0.0),
            // currentcolorの初期解決先(このデフォルト値自体が親を持たない場合の
            // 基準になる)。実際の解決は`resolve_color`が行う。
            border_top_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_right_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_bottom_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_left_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,
            border_top_left_radius: CornerRadius::default(),
            border_top_right_radius: CornerRadius::default(),
            border_bottom_right_radius: CornerRadius::default(),
            border_bottom_left_radius: CornerRadius::default(),
            font_size: Length(16.0),
            // 既定は「未指定」を表す空 Vec。`select_for_char`は空なら
            // 呼び出し側フォント(`--font`/`@font-face`)へフォールバックする
            // ([0036]決定3-1改訂: `sans-serif`は明示時のみゴシック解決)。
            font_family: Vec::new(),
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            background_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0.0,
            },
            background_image: None,
            background_position: BackgroundPosition::default(),
            background_size: BackgroundSize::default(),
            background_repeat: BackgroundRepeat::default(),
            background_attachment: BackgroundAttachment::default(),
            text_decoration_line: TextDecorationLine::default(),
            pseudo_before_content: None,
            pseudo_after_content: None,
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            break_inside: BreakInside::Auto,
            orphans: 2,
            widows: 2,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            top: LengthPercentageOrAuto::Auto,
            right: LengthPercentageOrAuto::Auto,
            bottom: LengthPercentageOrAuto::Auto,
            left: LengthPercentageOrAuto::Auto,
            text_align: TextAlign::Left,
            line_height: LineHeight::Normal,
            text_indent: zero_lp,
            white_space: WhiteSpace::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_transform: TextTransform::None,
            text_shadow: Vec::new(),
            text_overflow: TextOverflow::Clip,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            hyphens: Hyphens::Manual,
            text_emphasis_style: EmphasisStyle::None,
            text_emphasis_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            text_emphasis_position: EmphasisPosition::Over,
            grid_template_columns: TrackList::default(),
            grid_template_rows: TrackList::default(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_template_areas: Vec::new(),
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            // `justify-items`の初期値は`legacy`(実質`stretch`)、
            // `justify-self`は`auto`(親の`justify-items`に従う)。
            justify_items: AlignItems::Stretch,
            justify_self: AlignSelf::Auto,
            border_collapse: BorderCollapse::Separate,
            border_spacing_horizontal: Length(0.0),
            border_spacing_vertical: Length(0.0),
            caption_side: CaptionSide::Top,
            table_layout: TableLayout::Auto,
            empty_cells: EmptyCells::Show,
            vertical_align: VerticalAlign::Baseline,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: None,
            overflow: Overflow::Visible,
            box_sizing: BoxSizing::ContentBox,
            z_index: ZIndex::Auto,
            visibility: Visibility::Visible,
            outline_width: Length(0.0),
            outline_style: BorderStyle::None,
            outline_color: RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 1.0,
            },
            // 一般的なブラウザ既定値と同じ曲線引用符([0024]決定3)。
            quotes: Some(vec![
                QuotePair {
                    open: "\u{201C}".to_string(),
                    close: "\u{201D}".to_string(),
                },
                QuotePair {
                    open: "\u{2018}".to_string(),
                    close: "\u{2019}".to_string(),
                },
            ]),
            first_letter_style: None,
            object_fit: ObjectFit::Fill,
            object_position: BackgroundPosition {
                horizontal: LengthPercentage::Percentage(0.5),
                vertical: LengthPercentage::Percentage(0.5),
            },
            box_shadow: Vec::new(),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_content: AlignContent::Stretch,
            align_self: AlignSelf::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: FlexBasis::Auto,
            row_gap: LengthPercentage::Length(0.0),
            column_gap: LengthPercentage::Length(0.0),
            transform: Vec::new(),
            transform_origin: BackgroundPosition {
                horizontal: LengthPercentage::Percentage(0.5),
                vertical: LengthPercentage::Percentage(0.5),
            },
            opacity: 1.0,
        }
    }
}

/// DOM全体の計算スタイルを求める。要素以外のノード(テキスト等)には
/// ボックスに関するプロパティは意味を持たないため、単純に親の計算スタイルを
/// (= 継承プロパティも含めてそのまま)引き継ぐ。
///
/// `rem`の基準となるルート要素(`<html>`)のフォントサイズは、木を辿りながら
/// 最初に見つかった要素で確定する。それより前の(まだ確定していない)時点では
/// 初期値`16px`を仮の基準として使うが、ルート要素自身が`rem`単位で
/// 自分自身のfont-sizeを指定するような通常あり得ない記述を除けば、
/// ルート要素の子孫は必ずルート確定後に処理されるため実用上問題ない。
pub fn compute_styles(
    dom: &Dom,
    ua: &Stylesheet,
    author: &Stylesheet,
) -> HashMap<NodeId, ComputedStyle> {
    let mut styles = HashMap::new();
    let ctx = StyleContext {
        ua,
        author,
        root_font_size: Cell::new(ComputedStyle::default().font_size.0),
    };
    // カウンタ([0024]決定2)・quote深度(決定3)はいずれも文書全体で1つ
    // (バッチ処理は文書全体を1回の走査で処理するため、ここで新規に用意すれば足りる)。
    let mut counters = HashMap::new();
    let mut quote_depth = 0;
    compute_recursive(
        dom,
        dom.document(),
        None,
        false,
        &ctx,
        &mut counters,
        &mut quote_depth,
        &mut styles,
    );
    styles
}

/// [`compute_styles`]のバリアント: `dom`のルート(`document()`)から辿るの
/// ではなく、任意の`root`(とその子孫)を、既知の親スタイル`parent_style`・
/// 確定済みの`root_font_size`(`rem`の基準)を起点に計算する。
///
/// マイルストーン3のストリーミング処理で、`<body>`直下のトップレベル要素が
/// 確定するたびに、そのノードだけを対象に、事前に計算済みの`<body>`の
/// スタイルを引き継いでスタイル計算するために使う。`dom`自体は文書全体の
/// ものをそのまま渡してよい(`root`とその子孫だけが辿られる)。`root`は
/// `<html>`のようなルート候補ではないため、`rem`基準を上書きしない
/// (`is_root_candidate: false`で呼ぶ)。
///
/// `counters`/`quote_depth`はドキュメント順に依存する状態([0024]決定2・3)
/// なので、呼び出し側(ストリーミング処理では`Engine::StreamingState`)が
/// トップレベル要素をまたいで永続させ、都度`&mut`で渡すこと。
#[allow(clippy::too_many_arguments)]
pub fn compute_styles_with_parent(
    dom: &Dom,
    root: NodeId,
    parent_style: &ComputedStyle,
    root_font_size: f32,
    ua: &Stylesheet,
    author: &Stylesheet,
    counters: &mut HashMap<String, Vec<i32>>,
    quote_depth: &mut i32,
) -> HashMap<NodeId, ComputedStyle> {
    let mut styles = HashMap::new();
    let ctx = StyleContext {
        ua,
        author,
        root_font_size: Cell::new(root_font_size),
    };
    compute_recursive(
        dom,
        root,
        Some(parent_style),
        false,
        &ctx,
        counters,
        quote_depth,
        &mut styles,
    );
    styles
}

/// `element`単体の計算スタイルを、既知の親スタイルを起点に計算する。
///
/// マイルストーン3のストリーミング処理で、`<html>`/`<body>`要素自身の
/// スタイルを(それぞれの子孫全体を再帰的に辿ることなく)個別に確定させる
/// ために使う。[`compute_element_style`]をそのまま公開したもの
/// (この要素がpushしたカウンタ名の一覧はpop対象として追跡しない。
/// `<html>`/`<body>`レベルの`counter-reset`は文書全体に永続して構わないため)。
///
/// **既知の簡略化**: この要素の`::after`(`content`)は解決されない
/// (常に`None`)。`::after`の解決には子孫の処理完了後の状態が必要
/// (`compute_recursive`参照)だが、この関数は子孫を辿らないため。
/// `<html>`/`<body>`要素自身に`::after`生成コンテンツを使うケースは
/// 実務上稀と判断した。
#[allow(clippy::too_many_arguments)]
pub fn compute_single_element_style(
    dom: &Dom,
    element: NodeId,
    parent_style: Option<&ComputedStyle>,
    root_font_size: f32,
    ua: &Stylesheet,
    author: &Stylesheet,
    counters: &mut HashMap<String, Vec<i32>>,
    quote_depth: &mut i32,
) -> ComputedStyle {
    compute_element_style(
        dom,
        element,
        parent_style,
        root_font_size,
        ua,
        author,
        counters,
        quote_depth,
    )
    .0
}

/// `compute_recursive`/`compute_element_style`の再帰全体で共有する、
/// 木を辿る間変化しない(または`Cell`経由で一方向にのみ更新される)値。
/// 引数の数を抑えるための単純なまとめ役。
struct StyleContext<'a> {
    ua: &'a Stylesheet,
    author: &'a Stylesheet,
    /// `rem`の基準となるルート要素(`<html>`)の計算済みフォントサイズ。
    /// 木を辿りながら最初に見つかった要素で確定する。
    root_font_size: Cell<f32>,
}

/// 戻り値は`node`自身が`counter-reset`(または暗黙生成)でpushしたカウンタ名の
/// 一覧([0024]決定2)。CSSの仕様上、この push が作るスコープは「`node`自身と
/// それに続く兄弟要素」(`node`の**親**の子要素列の残り)まで及ぶため、
/// popできるのは`node`の親であって`node`自身ではない。よって`node`はここでは
/// popせず、そのまま呼び出し元(親の`compute_recursive`)へ返す。一方、
/// `node`の直接の子が同様にpushしたカウンタは、`node`の子要素列の走査
/// (=兄弟スコープ)が終わるこの関数の末尾でpopしてよい。
#[allow(clippy::too_many_arguments)]
fn compute_recursive(
    dom: &Dom,
    node: NodeId,
    parent_style: Option<&ComputedStyle>,
    is_root_candidate: bool,
    ctx: &StyleContext<'_>,
    counters: &mut HashMap<String, Vec<i32>>,
    quote_depth: &mut i32,
    out: &mut HashMap<NodeId, ComputedStyle>,
) -> Vec<String> {
    let (mut style, own_pushed_counter_names, after_parts) = match &dom.node(node).data {
        NodeData::Element { .. } => {
            let (style, pushed_counter_names, after_parts) = compute_element_style(
                dom,
                node,
                parent_style,
                ctx.root_font_size.get(),
                ctx.ua,
                ctx.author,
                counters,
                quote_depth,
            );
            // ドキュメント直下の最初の要素(通常は<html>)がルート要素。
            if is_root_candidate {
                ctx.root_font_size.set(style.font_size.0);
            }
            (style, pushed_counter_names, after_parts)
        }
        _ => (parent_style.cloned().unwrap_or_default(), Vec::new(), None),
    };

    // `node`がドキュメントノードであれば、その直下の子(通常は<html>)がルート要素候補。
    let children_are_root_candidates = node == dom.document();
    let mut children_pushed_counter_names = Vec::new();
    for child in dom.children(node) {
        let pushed_by_child = compute_recursive(
            dom,
            child,
            Some(&style),
            children_are_root_candidates,
            ctx,
            counters,
            quote_depth,
            out,
        );
        children_pushed_counter_names.extend(pushed_by_child);
    }

    // 直接の子(とその兄弟スコープ全体)がpushしたカウンタのスコープを終了する。
    // `node`自身がpushした分はここではpopしない(呼び出し元=`node`の親が
    // popする、上記のコメント参照)。
    for name in &children_pushed_counter_names {
        if let Some(stack) = counters.get_mut(name) {
            stack.pop();
        }
    }

    // `::after`のcontentを、子孫の処理が終わった今の状態(counter()/quotesが
    // 子孫による変更を反映済み)で解決する。
    let quotes = style.quotes.clone();
    style.pseudo_after_content =
        resolve_content_parts(after_parts, dom, node, counters, quote_depth, &quotes);

    out.insert(node, style);
    own_pushed_counter_names
}

/// `element`の計算スタイルを求める。戻り値は`(スタイル, pushしたカウンタ名の
/// 一覧, 未解決の::after content)`の3つ組。
///
/// `Vec<String>`は、この要素が`counter-reset`(または未定義カウンタへの
/// `counter-increment`による暗黙生成、[0024](
/// ../../../docs/decisions/0024-generated-content-design.md)決定2)で
/// `counters`にpushしたカウンタ名の一覧で、呼び出し側(`compute_recursive`)が
/// 子孫の処理後に同じ数だけpopするために使う。
///
/// `::after`の`content`は、この関数の時点では解決せず`Option<Vec<ContentPart>>`
/// のまま返す。`::after`はDOM順で子孫より後に現れるため、`counter()`/`quotes`
/// (子孫による変更を反映すべき)は子孫の処理が終わってから解決する必要がある
/// (呼び出し側の`compute_recursive`が子ループの後に`resolve_content_parts`を
/// 呼び、`ComputedStyle::pseudo_after_content`を埋める)。
#[allow(clippy::too_many_arguments)]
fn compute_element_style(
    dom: &Dom,
    element: NodeId,
    parent: Option<&ComputedStyle>,
    root_font_size: f32,
    ua: &Stylesheet,
    author: &Stylesheet,
    counters: &mut HashMap<String, Vec<i32>>,
    quote_depth: &mut i32,
) -> (ComputedStyle, Vec<String>, Option<Vec<ContentPart>>) {
    let (ua_declarations, author_declarations) =
        matching_declarations_by_origin(dom, element, ua, author);
    let inline_declarations = inline_style_declarations(dom, element);
    // レガシー表示属性(`bgcolor`/`align`等)と`data-page-break`糖衣は、
    // UAスタイルシートより強く作者CSSより弱い位置に置く([0039](
    // ../../../docs/decisions/0039-presentational-attributes-design.md)決定1)。
    let mut attribute_declarations = presentational_hint_declarations(dom, element);
    attribute_declarations.extend(data_page_break_declarations(dom, element));

    let mut display = None;
    let mut width = None;
    let mut height = None;
    let mut margin_top = None;
    let mut margin_right = None;
    let mut margin_bottom = None;
    let mut margin_left = None;
    let mut padding_top = None;
    let mut padding_right = None;
    let mut padding_bottom = None;
    let mut padding_left = None;
    let mut border_top_width = None;
    let mut border_right_width = None;
    let mut border_bottom_width = None;
    let mut border_left_width = None;
    let mut border_top_color = None;
    let mut border_right_color = None;
    let mut border_bottom_color = None;
    let mut border_left_color = None;
    let mut border_top_style = None;
    let mut border_right_style = None;
    let mut border_bottom_style = None;
    let mut border_left_style = None;
    let mut border_top_left_radius = None;
    let mut border_top_right_radius = None;
    let mut border_bottom_right_radius = None;
    let mut border_bottom_left_radius = None;
    let mut font_size = None;
    let mut font_family = None;
    let mut font_weight = None;
    let mut font_style = None;
    let mut color = None;
    let mut background_color = None;
    let mut background_image = None;
    let mut background_position = None;
    let mut background_size = None;
    let mut background_repeat = None;
    let mut background_attachment = None;
    let mut text_decoration_line = None;
    let mut break_before = None;
    let mut break_after = None;
    let mut break_inside = None;
    let mut orphans = None;
    let mut widows = None;
    let mut float = None;
    let mut clear = None;
    let mut position = None;
    let mut top = None;
    let mut right = None;
    let mut bottom = None;
    let mut left = None;
    let mut text_align = None;
    let mut line_height = None;
    let mut text_indent = None;
    let mut white_space = None;
    let mut letter_spacing = None;
    let mut word_spacing = None;
    let mut text_transform = None;
    let mut text_shadow = None;
    let mut text_overflow = None;
    let mut word_break = None;
    let mut overflow_wrap = None;
    let mut hyphens = None;
    let mut text_emphasis_style = None;
    let mut text_emphasis_color = None;
    let mut text_emphasis_position = None;
    let mut grid_template_columns = None;
    let mut grid_template_rows = None;
    let mut grid_auto_columns = None;
    let mut grid_auto_rows = None;
    let mut grid_auto_flow = None;
    let mut grid_template_areas = None;
    let mut grid_row_start = None;
    let mut grid_row_end = None;
    let mut grid_column_start = None;
    let mut grid_column_end = None;
    let mut justify_items = None;
    let mut justify_self = None;
    let mut border_collapse = None;
    let mut border_spacing = None;
    let mut caption_side = None;
    let mut table_layout = None;
    let mut empty_cells = None;
    let mut vertical_align = None;
    let mut list_style_type = None;
    let mut list_style_position = None;
    let mut list_style_image = None;
    let mut overflow = None;
    let mut box_sizing = None;
    let mut z_index = None;
    let mut visibility = None;
    let mut outline_width = None;
    let mut outline_style = None;
    let mut outline_color = None;
    let mut counter_reset = None;
    let mut min_width = None;
    let mut min_height = None;
    let mut max_width = None;
    let mut max_height = None;
    let mut aspect_ratio = None;
    let mut counter_increment = None;
    let mut quotes = None;
    let mut object_fit = None;
    let mut object_position = None;
    let mut box_shadow = None;
    let mut flex_direction = None;
    let mut flex_wrap = None;
    let mut justify_content = None;
    let mut align_items = None;
    let mut align_content = None;
    let mut align_self = None;
    let mut flex_grow = None;
    let mut flex_shrink = None;
    let mut flex_basis = None;
    let mut row_gap = None;
    let mut column_gap = None;
    let mut transform = None;
    let mut transform_origin = None;
    let mut opacity = None;

    // カスケード順(優先度昇順)に走査するので、後で見つかったものが自然に勝つ。
    // HTML属性由来の宣言(レガシー表示属性・`data-page-break`糖衣)は
    // 「UAスタイルシートより強く、作者CSSでは上書きできる既定のヒント」という
    // 位置づけなので両者の間に置く([0039]決定1)。インラインstyle属性は
    // セレクタベースのどの宣言よりも優先度が高いため、最後に置く。
    for decl in ua_declarations
        .into_iter()
        .chain(attribute_declarations.iter())
        .chain(author_declarations)
        .chain(inline_declarations.iter())
    {
        match decl {
            PropertyDeclaration::Display(v) => display = Some(*v),
            PropertyDeclaration::Width(v) => width = Some(*v),
            PropertyDeclaration::Height(v) => height = Some(*v),
            PropertyDeclaration::MinWidth(v) => min_width = Some(*v),
            PropertyDeclaration::MinHeight(v) => min_height = Some(*v),
            PropertyDeclaration::MaxWidth(v) => max_width = Some(*v),
            PropertyDeclaration::MaxHeight(v) => max_height = Some(*v),
            PropertyDeclaration::AspectRatio(v) => aspect_ratio = Some(*v),
            PropertyDeclaration::MarginTop(v) => margin_top = Some(*v),
            PropertyDeclaration::MarginRight(v) => margin_right = Some(*v),
            PropertyDeclaration::MarginBottom(v) => margin_bottom = Some(*v),
            PropertyDeclaration::MarginLeft(v) => margin_left = Some(*v),
            PropertyDeclaration::PaddingTop(v) => padding_top = Some(*v),
            PropertyDeclaration::PaddingRight(v) => padding_right = Some(*v),
            PropertyDeclaration::PaddingBottom(v) => padding_bottom = Some(*v),
            PropertyDeclaration::PaddingLeft(v) => padding_left = Some(*v),
            PropertyDeclaration::BorderTopWidth(v) => border_top_width = Some(*v),
            PropertyDeclaration::BorderRightWidth(v) => border_right_width = Some(*v),
            PropertyDeclaration::BorderBottomWidth(v) => border_bottom_width = Some(*v),
            PropertyDeclaration::BorderLeftWidth(v) => border_left_width = Some(*v),
            PropertyDeclaration::BorderTopColor(v) => border_top_color = Some(*v),
            PropertyDeclaration::BorderRightColor(v) => border_right_color = Some(*v),
            PropertyDeclaration::BorderBottomColor(v) => border_bottom_color = Some(*v),
            PropertyDeclaration::BorderLeftColor(v) => border_left_color = Some(*v),
            PropertyDeclaration::BorderTopStyle(v) => border_top_style = Some(*v),
            PropertyDeclaration::BorderRightStyle(v) => border_right_style = Some(*v),
            PropertyDeclaration::BorderBottomStyle(v) => border_bottom_style = Some(*v),
            PropertyDeclaration::BorderLeftStyle(v) => border_left_style = Some(*v),
            PropertyDeclaration::BorderTopLeftRadius(v) => border_top_left_radius = Some(*v),
            PropertyDeclaration::BorderTopRightRadius(v) => border_top_right_radius = Some(*v),
            PropertyDeclaration::BorderBottomRightRadius(v) => {
                border_bottom_right_radius = Some(*v)
            }
            PropertyDeclaration::BorderBottomLeftRadius(v) => border_bottom_left_radius = Some(*v),
            PropertyDeclaration::FontSize(v) => font_size = Some(*v),
            PropertyDeclaration::FontFamily(v) => font_family = Some(v.clone()),
            PropertyDeclaration::FontWeight(v) => font_weight = Some(*v),
            PropertyDeclaration::FontStyle(v) => font_style = Some(*v),
            PropertyDeclaration::Color(v) => color = Some(*v),
            PropertyDeclaration::BackgroundColor(v) => background_color = Some(*v),
            PropertyDeclaration::BackgroundImage(v) => background_image = v.clone(),
            PropertyDeclaration::BackgroundPosition(v) => background_position = Some(*v),
            PropertyDeclaration::BackgroundSize(v) => background_size = Some(*v),
            PropertyDeclaration::BackgroundRepeat(v) => background_repeat = Some(*v),
            PropertyDeclaration::BackgroundAttachment(v) => background_attachment = Some(*v),
            PropertyDeclaration::TextDecorationLine(v) => text_decoration_line = Some(*v),
            // `content`は`::before`/`::after`専用で、通常の要素では効果を持たない
            // (`matching_pseudo_content`が別途、擬似要素向けのマッチングを行う)。
            PropertyDeclaration::Content(_) => {}
            PropertyDeclaration::BreakBefore(v) => break_before = Some(*v),
            PropertyDeclaration::BreakAfter(v) => break_after = Some(*v),
            PropertyDeclaration::BreakInside(v) => break_inside = Some(*v),
            PropertyDeclaration::Orphans(v) => orphans = Some(*v),
            PropertyDeclaration::Widows(v) => widows = Some(*v),
            PropertyDeclaration::Float(v) => float = Some(*v),
            PropertyDeclaration::Clear(v) => clear = Some(*v),
            PropertyDeclaration::Position(v) => position = Some(*v),
            PropertyDeclaration::Top(v) => top = Some(*v),
            PropertyDeclaration::Right(v) => right = Some(*v),
            PropertyDeclaration::Bottom(v) => bottom = Some(*v),
            PropertyDeclaration::Left(v) => left = Some(*v),
            PropertyDeclaration::TextAlign(v) => text_align = Some(*v),
            PropertyDeclaration::LineHeight(v) => line_height = Some(*v),
            PropertyDeclaration::TextIndent(v) => text_indent = Some(*v),
            PropertyDeclaration::WhiteSpace(v) => white_space = Some(*v),
            PropertyDeclaration::LetterSpacing(v) => letter_spacing = Some(*v),
            PropertyDeclaration::WordSpacing(v) => word_spacing = Some(*v),
            PropertyDeclaration::TextTransform(v) => text_transform = Some(*v),
            PropertyDeclaration::TextShadow(v) => text_shadow = Some(v.clone()),
            PropertyDeclaration::TextOverflow(v) => text_overflow = Some(*v),
            PropertyDeclaration::WordBreak(v) => word_break = Some(*v),
            PropertyDeclaration::OverflowWrap(v) => overflow_wrap = Some(*v),
            PropertyDeclaration::Hyphens(v) => hyphens = Some(*v),
            PropertyDeclaration::TextEmphasisStyle(v) => text_emphasis_style = Some(v.clone()),
            PropertyDeclaration::TextEmphasisColor(v) => text_emphasis_color = Some(*v),
            PropertyDeclaration::TextEmphasisPosition(v) => text_emphasis_position = Some(*v),
            PropertyDeclaration::GridTemplateColumns(v) => grid_template_columns = Some(v.clone()),
            PropertyDeclaration::GridTemplateRows(v) => grid_template_rows = Some(v.clone()),
            PropertyDeclaration::GridAutoColumns(v) => grid_auto_columns = Some(v.clone()),
            PropertyDeclaration::GridAutoRows(v) => grid_auto_rows = Some(v.clone()),
            PropertyDeclaration::GridAutoFlow(v) => grid_auto_flow = Some(*v),
            PropertyDeclaration::GridTemplateAreas(v) => grid_template_areas = Some(v.clone()),
            PropertyDeclaration::GridRowStart(v) => grid_row_start = Some(v.clone()),
            PropertyDeclaration::GridRowEnd(v) => grid_row_end = Some(v.clone()),
            PropertyDeclaration::GridColumnStart(v) => grid_column_start = Some(v.clone()),
            PropertyDeclaration::GridColumnEnd(v) => grid_column_end = Some(v.clone()),
            PropertyDeclaration::JustifyItems(v) => justify_items = Some(*v),
            PropertyDeclaration::JustifySelf(v) => justify_self = Some(*v),
            PropertyDeclaration::BorderCollapse(v) => border_collapse = Some(*v),
            PropertyDeclaration::BorderSpacing(h, v) => border_spacing = Some((*h, *v)),
            PropertyDeclaration::CaptionSide(v) => caption_side = Some(*v),
            PropertyDeclaration::TableLayout(v) => table_layout = Some(*v),
            PropertyDeclaration::EmptyCells(v) => empty_cells = Some(*v),
            PropertyDeclaration::VerticalAlign(v) => vertical_align = Some(*v),
            PropertyDeclaration::ListStyleType(v) => list_style_type = Some(*v),
            PropertyDeclaration::ListStylePosition(v) => list_style_position = Some(*v),
            PropertyDeclaration::ListStyleImage(v) => list_style_image = Some(v.clone()),
            PropertyDeclaration::Overflow(v) => overflow = Some(*v),
            PropertyDeclaration::BoxSizing(v) => box_sizing = Some(*v),
            PropertyDeclaration::ZIndex(v) => z_index = Some(*v),
            PropertyDeclaration::Visibility(v) => visibility = Some(*v),
            PropertyDeclaration::OutlineWidth(v) => outline_width = Some(*v),
            PropertyDeclaration::OutlineStyle(v) => outline_style = Some(*v),
            PropertyDeclaration::OutlineColor(v) => outline_color = Some(*v),
            PropertyDeclaration::CounterReset(v) => counter_reset = Some(v.clone()),
            PropertyDeclaration::CounterIncrement(v) => counter_increment = Some(v.clone()),
            PropertyDeclaration::Quotes(v) => quotes = Some(v.clone()),
            PropertyDeclaration::ObjectFit(v) => object_fit = Some(*v),
            PropertyDeclaration::ObjectPosition(v) => object_position = Some(*v),
            PropertyDeclaration::BoxShadow(v) => box_shadow = Some(v.clone()),
            PropertyDeclaration::FlexDirection(v) => flex_direction = Some(*v),
            PropertyDeclaration::FlexWrap(v) => flex_wrap = Some(*v),
            PropertyDeclaration::JustifyContent(v) => justify_content = Some(*v),
            PropertyDeclaration::AlignItems(v) => align_items = Some(*v),
            PropertyDeclaration::AlignContent(v) => align_content = Some(*v),
            PropertyDeclaration::AlignSelf(v) => align_self = Some(*v),
            PropertyDeclaration::FlexGrow(v) => flex_grow = Some(*v),
            PropertyDeclaration::FlexShrink(v) => flex_shrink = Some(*v),
            PropertyDeclaration::FlexBasis(v) => flex_basis = Some(*v),
            PropertyDeclaration::RowGap(v) => row_gap = Some(*v),
            PropertyDeclaration::ColumnGap(v) => column_gap = Some(*v),
            PropertyDeclaration::Transform(v) => transform = Some(v.clone()),
            PropertyDeclaration::TransformOrigin(v) => transform_origin = Some(*v),
            PropertyDeclaration::Opacity(v) => opacity = Some(*v),
        }
    }

    let initial = ComputedStyle::default();
    let inherited_font_size = parent.map_or(initial.font_size, |p| p.font_size);
    let inherited_font_family =
        parent.map_or_else(|| initial.font_family.clone(), |p| p.font_family.clone());
    let inherited_font_weight = parent.map_or(initial.font_weight, |p| p.font_weight);
    let inherited_font_style = parent.map_or(initial.font_style, |p| p.font_style);
    let inherited_color = parent.map_or(initial.color, |p| p.color);
    let inherited_text_decoration_line =
        parent.map_or(initial.text_decoration_line, |p| p.text_decoration_line);
    let inherited_text_align = parent.map_or(initial.text_align, |p| p.text_align);
    let inherited_line_height = parent.map_or(initial.line_height, |p| p.line_height);
    let inherited_text_indent = parent.map_or(initial.text_indent, |p| p.text_indent);
    let inherited_white_space = parent.map_or(initial.white_space, |p| p.white_space);
    let inherited_letter_spacing = parent.map_or(initial.letter_spacing, |p| p.letter_spacing);
    let inherited_word_spacing = parent.map_or(initial.word_spacing, |p| p.word_spacing);
    let inherited_text_transform = parent.map_or(initial.text_transform, |p| p.text_transform);
    let inherited_word_break = parent.map_or(initial.word_break, |p| p.word_break);
    let inherited_overflow_wrap = parent.map_or(initial.overflow_wrap, |p| p.overflow_wrap);
    let inherited_hyphens = parent.map_or(initial.hyphens, |p| p.hyphens);
    let inherited_emphasis_position =
        parent.map_or(initial.text_emphasis_position, |p| p.text_emphasis_position);
    let inherited_emphasis_style = parent
        .map(|p| p.text_emphasis_style.clone())
        .unwrap_or_else(|| initial.text_emphasis_style.clone());
    let inherited_border_collapse = parent.map_or(initial.border_collapse, |p| p.border_collapse);
    let inherited_border_spacing_horizontal = parent
        .map_or(initial.border_spacing_horizontal, |p| {
            p.border_spacing_horizontal
        });
    let inherited_border_spacing_vertical = parent.map_or(initial.border_spacing_vertical, |p| {
        p.border_spacing_vertical
    });
    let inherited_caption_side = parent.map_or(initial.caption_side, |p| p.caption_side);
    let inherited_empty_cells = parent.map_or(initial.empty_cells, |p| p.empty_cells);
    let inherited_list_style_type = parent.map_or(initial.list_style_type, |p| p.list_style_type);
    let inherited_list_style_position =
        parent.map_or(initial.list_style_position, |p| p.list_style_position);
    let inherited_list_style_image = parent.map_or_else(
        || initial.list_style_image.clone(),
        |p| p.list_style_image.clone(),
    );
    let inherited_visibility = parent.map_or(initial.visibility, |p| p.visibility);
    let inherited_quotes = parent.map_or_else(|| initial.quotes.clone(), |p| p.quotes.clone());

    // font-sizeは他の長さ系プロパティより先に解決する。`em`の基準は仕様上
    // 「親要素の計算済みfont-size」(自分自身の値ではない、循環を避けるため)。
    let resolved_font_size = font_size
        .map(|specified| specified.resolve(inherited_font_size.0, root_font_size))
        .unwrap_or(inherited_font_size);
    // font-size以外の長さ系プロパティの`em`基準は、この要素自身の(今解決した)font-size。
    let own_font_size = resolved_font_size.0;
    let resolve_lp_or_auto = |v: Option<SpecifiedLengthPercentageOrAuto>,
                              initial: LengthPercentageOrAuto| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolve_lp = |v: Option<SpecifiedLengthPercentage>, initial: LengthPercentage| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolve_max_size = |v: Option<SpecifiedMaxSize>| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(MaxSize::None)
    };
    let resolve_len = |v: Option<SpecifiedLength>, initial: Length| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolve_corner_radius = |v: Option<SpecifiedCornerRadius>, initial: CornerRadius| {
        v.map(|specified| specified.resolve(own_font_size, root_font_size))
            .unwrap_or(initial)
    };
    let resolved_background_position = background_position
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.background_position);
    let resolved_background_size = background_size
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.background_size);

    // `line-height`の`<number>`/`<percentage>`は未乗算のまま継承する
    // ([0020]決定3)。`<percentage>`は既にfraction(50%→0.5)としてパース済みの
    // ため`<number>`と同じ扱いでよい。
    let resolved_line_height = match line_height {
        Some(SpecifiedLineHeight::Normal) => LineHeight::Normal,
        Some(SpecifiedLineHeight::Number(n) | SpecifiedLineHeight::Percentage(n)) => {
            LineHeight::Number(n)
        }
        Some(SpecifiedLineHeight::Length(l)) => {
            LineHeight::Length(l.resolve(own_font_size, root_font_size).0)
        }
        None => inherited_line_height,
    };
    let resolved_letter_spacing = letter_spacing
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(inherited_letter_spacing);
    let resolved_word_spacing = word_spacing
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(inherited_word_spacing);
    let resolved_border_spacing_horizontal = border_spacing
        .map(|(h, _)| resolve_len(Some(h), inherited_border_spacing_horizontal))
        .unwrap_or(inherited_border_spacing_horizontal);
    let resolved_border_spacing_vertical = border_spacing
        .map(|(_, v)| resolve_len(Some(v), inherited_border_spacing_vertical))
        .unwrap_or(inherited_border_spacing_vertical);

    let resolved_color = resolve_color(color, inherited_color);
    let resolved_background_color = match background_color {
        Some(Color::Rgba {
            red,
            green,
            blue,
            alpha,
        }) => RgbaColor {
            red,
            green,
            blue,
            alpha,
        },
        // `background-color: currentcolor`は、この要素自身の計算済みcolorを使う。
        Some(Color::CurrentColor) => resolved_color,
        None => initial.background_color,
    };
    // `border-color`の初期値は仕様上`currentcolor`なので、未指定時も
    // (`currentcolor`指定時と同様に)この要素自身の計算済みcolorへ解決する。
    let resolved_border_top_color = resolve_color(border_top_color, resolved_color);
    let resolved_border_right_color = resolve_color(border_right_color, resolved_color);
    let resolved_border_bottom_color = resolve_color(border_bottom_color, resolved_color);
    let resolved_border_left_color = resolve_color(border_left_color, resolved_color);
    // `outline-color`の初期値も`currentcolor`(仕様通り)。
    let resolved_outline_color = resolve_color(outline_color, resolved_color);

    let resolved_object_position = object_position
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.object_position);
    // `text-shadow`は継承プロパティなので、宣言が無ければ親の**解決済み**の値を
    // そのまま引き継ぐ(色は親の`color`で解決済みのまま。CSS仕様でも
    // `currentcolor`は継承時点の値で固定される)。
    let resolved_text_shadow: Vec<ComputedTextShadow> = match text_shadow {
        Some(shadows) => shadows
            .into_iter()
            .map(|specified| {
                let resolved = specified.resolve(own_font_size, root_font_size);
                ComputedTextShadow {
                    offset_x: resolved.offset_x,
                    offset_y: resolved.offset_y,
                    blur_radius: resolved.blur_radius.max(0.0),
                    color: resolve_color(resolved.color, resolved_color),
                }
            })
            .collect(),
        None => parent.map(|p| p.text_shadow.clone()).unwrap_or_default(),
    };

    // `text-emphasis-color`も継承プロパティ。初期値は`currentcolor`
    // (=この要素自身の`color`)。
    let resolved_text_emphasis_color = match text_emphasis_color {
        Some(color) => resolve_color(Some(color), resolved_color),
        None => parent.map_or(resolved_color, |p| p.text_emphasis_color),
    };

    // `box-shadow`のカンマ区切り各要素のem/rem解決と`currentcolor`解決
    // ([0032](../../../docs/decisions/0032-box-shadow-design.md)決定1)。
    // `blur-radius`は仕様上負値は無効だが、パース時点では拒否せず
    // ここで0未満をクランプする(簡易な頑健性、[0025]系の既存パターンに倣う)。
    let resolved_box_shadow: Vec<ComputedBoxShadow> = box_shadow
        .unwrap_or_default()
        .into_iter()
        .map(|specified| {
            let resolved = specified.resolve(own_font_size, root_font_size);
            ComputedBoxShadow {
                offset_x: resolved.offset_x,
                offset_y: resolved.offset_y,
                blur_radius: resolved.blur_radius.max(0.0),
                spread_radius: resolved.spread_radius,
                color: resolve_color(resolved.color, resolved_color),
                inset: resolved.inset,
            }
        })
        .collect();

    // flexbox関連([0034](../../../docs/decisions/0034-flexbox-design.md))。
    // いずれも非継承プロパティなので初期値との比較に`inherited_*`は不要。
    let resolved_flex_basis = flex_basis
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.flex_basis);
    let resolved_row_gap = row_gap
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.row_gap);
    let resolved_column_gap = column_gap
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.column_gap);

    // `transform`/`transform-origin`/`opacity`([0035](
    // ../../../docs/decisions/0035-opacity-transform-design.md))。いずれも
    // 非継承プロパティ。
    let resolved_transform = transform
        .map(|specified| {
            specified
                .into_iter()
                .map(|f| f.resolve(own_font_size, root_font_size))
                .collect()
        })
        .unwrap_or_else(|| initial.transform.clone());
    let resolved_transform_origin = transform_origin
        .map(|specified| specified.resolve(own_font_size, root_font_size))
        .unwrap_or(initial.transform_origin);
    let resolved_opacity = opacity.unwrap_or(initial.opacity);

    let resolved_float = float.unwrap_or(initial.float);
    let resolved_position = position.unwrap_or(initial.position);
    // CSS2.1 9.7: floatが`none`以外、または`position: absolute`/`fixed`なら
    // 要素は自動的にblock-levelとして計算される(`display: inline`でも)。
    // これにより`box_tree.rs::child_kind`の`Block`分岐がそのまま機能し、
    // インライン要素(`<span style="position: absolute">`)も`box_tree`の
    // Blocksループで捕捉できる([0019]決定2・[0049](
    // ../../../docs/decisions/0049-absolute-fixed-positioning-design.md))。
    let resolved_display = match display.unwrap_or(initial.display) {
        Display::Inline if resolved_float != Float::None || resolved_position.is_out_of_flow() => {
            Display::Block
        }
        other => other,
    };

    let resolved_quotes = quotes.unwrap_or(inherited_quotes);

    // `counter-reset`/`counter-increment`の適用([0024]決定2)。`content`の
    // `counter()`/`counters()`解決より先に行う必要がある(この要素自身が
    // reset/incrementした値を、この要素の`content`が参照できるようにするため)。
    let mut pushed_counter_names = Vec::new();
    if let Some(resets) = &counter_reset {
        for (name, value) in resets {
            counters.entry(name.clone()).or_default().push(*value);
            pushed_counter_names.push(name.clone());
        }
    }
    if let Some(increments) = &counter_increment {
        for (name, value) in increments {
            let stack = counters.entry(name.clone()).or_default();
            if stack.is_empty() {
                // スコープ内にその名前のカウンタが一つも無い場合の暗黙生成
                // (簡略化: 本来はドキュメント全体に永続すべきだが、この要素の
                // 部分木を抜けたらpopされる、決定2の既知の簡略化)。
                stack.push(0);
                pushed_counter_names.push(name.clone());
            }
            *stack.last_mut().expect("just ensured non-empty") += value;
        }
    }

    // `content`(::before)の解決。カウンタ・quote深度の「今の状態」を見る
    // 必要があるため、上のreset/increment適用より後で行う。`::before`は
    // 子孫より先にDOM順で現れるため、ここ(子孫を辿る前)で解決してよい。
    //
    // `::after`は逆に子孫より後にDOM順で現れるため、ここでは解決しない
    // (`counter()`/`quotes`の状態は子孫による変更を反映すべき)。パーツの列を
    // 未解決のまま呼び出し元(`compute_recursive`)へ返し、子孫の処理が
    // 終わった後に解決してもらう。
    let before_parts = matching_pseudo_content(dom, element, PseudoElement::Before, ua, author);
    let after_parts = matching_pseudo_content(dom, element, PseudoElement::After, ua, author);
    let pseudo_before_content = resolve_content_parts(
        before_parts,
        dom,
        element,
        counters,
        quote_depth,
        &resolved_quotes,
    );

    // `::first-letter`(決定4)。対応プロパティのみの限定的な上書きスタイル。
    let first_letter_declarations =
        matching_pseudo_declarations(dom, element, PseudoElement::FirstLetter, ua, author);
    let first_letter_style = compute_first_letter_style(
        &first_letter_declarations,
        resolved_font_size.0,
        root_font_size,
    );

    let style = ComputedStyle {
        display: resolved_display,
        width: resolve_lp_or_auto(width, initial.width),
        height: resolve_lp_or_auto(height, initial.height),
        min_width: resolve_lp(min_width, initial.min_width),
        min_height: resolve_lp(min_height, initial.min_height),
        max_width: resolve_max_size(max_width),
        max_height: resolve_max_size(max_height),
        aspect_ratio: aspect_ratio.unwrap_or_default(),
        margin_top: resolve_lp_or_auto(margin_top, initial.margin_top),
        margin_right: resolve_lp_or_auto(margin_right, initial.margin_right),
        margin_bottom: resolve_lp_or_auto(margin_bottom, initial.margin_bottom),
        margin_left: resolve_lp_or_auto(margin_left, initial.margin_left),
        padding_top: resolve_lp(padding_top, initial.padding_top),
        padding_right: resolve_lp(padding_right, initial.padding_right),
        padding_bottom: resolve_lp(padding_bottom, initial.padding_bottom),
        padding_left: resolve_lp(padding_left, initial.padding_left),
        border_top_width: resolve_len(border_top_width, initial.border_top_width),
        border_right_width: resolve_len(border_right_width, initial.border_right_width),
        border_bottom_width: resolve_len(border_bottom_width, initial.border_bottom_width),
        border_left_width: resolve_len(border_left_width, initial.border_left_width),
        border_top_color: resolved_border_top_color,
        border_right_color: resolved_border_right_color,
        border_bottom_color: resolved_border_bottom_color,
        border_left_color: resolved_border_left_color,
        border_top_style: border_top_style.unwrap_or(initial.border_top_style),
        border_right_style: border_right_style.unwrap_or(initial.border_right_style),
        border_bottom_style: border_bottom_style.unwrap_or(initial.border_bottom_style),
        border_left_style: border_left_style.unwrap_or(initial.border_left_style),
        border_top_left_radius: resolve_corner_radius(
            border_top_left_radius,
            initial.border_top_left_radius,
        ),
        border_top_right_radius: resolve_corner_radius(
            border_top_right_radius,
            initial.border_top_right_radius,
        ),
        border_bottom_right_radius: resolve_corner_radius(
            border_bottom_right_radius,
            initial.border_bottom_right_radius,
        ),
        border_bottom_left_radius: resolve_corner_radius(
            border_bottom_left_radius,
            initial.border_bottom_left_radius,
        ),
        font_size: resolved_font_size,
        font_family: font_family.unwrap_or(inherited_font_family),
        font_weight: font_weight.unwrap_or(inherited_font_weight),
        font_style: font_style.unwrap_or(inherited_font_style),
        color: resolved_color,
        background_color: resolved_background_color,
        background_image: background_image.or(initial.background_image),
        background_position: resolved_background_position,
        background_size: resolved_background_size,
        background_repeat: background_repeat.unwrap_or(initial.background_repeat),
        background_attachment: background_attachment.unwrap_or(initial.background_attachment),
        text_decoration_line: text_decoration_line.unwrap_or(inherited_text_decoration_line),
        pseudo_before_content,
        // 子孫の処理後に`compute_recursive`が解決してこのフィールドを埋める
        // (未解決の`after_parts`は戻り値の3つ目として返す)。
        pseudo_after_content: None,
        break_before: break_before.unwrap_or(initial.break_before),
        break_after: break_after.unwrap_or(initial.break_after),
        break_inside: break_inside.unwrap_or(initial.break_inside),
        orphans: orphans.unwrap_or(initial.orphans),
        widows: widows.unwrap_or(initial.widows),
        float: resolved_float,
        clear: clear.unwrap_or(initial.clear),
        position: resolved_position,
        top: resolve_lp_or_auto(top, initial.top),
        right: resolve_lp_or_auto(right, initial.right),
        bottom: resolve_lp_or_auto(bottom, initial.bottom),
        left: resolve_lp_or_auto(left, initial.left),
        text_align: text_align.unwrap_or(inherited_text_align),
        line_height: resolved_line_height,
        text_indent: resolve_lp(text_indent, inherited_text_indent),
        white_space: white_space.unwrap_or(inherited_white_space),
        letter_spacing: resolved_letter_spacing,
        word_spacing: resolved_word_spacing,
        text_transform: text_transform.unwrap_or(inherited_text_transform),
        text_shadow: resolved_text_shadow,
        text_overflow: text_overflow.unwrap_or_default(),
        word_break: word_break.unwrap_or(inherited_word_break),
        overflow_wrap: overflow_wrap.unwrap_or(inherited_overflow_wrap),
        hyphens: hyphens.unwrap_or(inherited_hyphens),
        text_emphasis_style: text_emphasis_style.unwrap_or(inherited_emphasis_style),
        text_emphasis_color: resolved_text_emphasis_color,
        text_emphasis_position: text_emphasis_position.unwrap_or(inherited_emphasis_position),
        grid_template_columns: grid_template_columns
            .map(|list| list.resolve(own_font_size, root_font_size))
            .unwrap_or_default(),
        grid_template_rows: grid_template_rows
            .map(|list| list.resolve(own_font_size, root_font_size))
            .unwrap_or_default(),
        grid_auto_columns: grid_auto_columns
            .map(|sizes| resolve_track_sizes(&sizes, own_font_size, root_font_size))
            .unwrap_or_default(),
        grid_auto_rows: grid_auto_rows
            .map(|sizes| resolve_track_sizes(&sizes, own_font_size, root_font_size))
            .unwrap_or_default(),
        grid_auto_flow: grid_auto_flow.unwrap_or(initial.grid_auto_flow),
        grid_template_areas: grid_template_areas.unwrap_or_default(),
        grid_row_start: grid_row_start.unwrap_or(GridLine::Auto),
        grid_row_end: grid_row_end.unwrap_or(GridLine::Auto),
        grid_column_start: grid_column_start.unwrap_or(GridLine::Auto),
        grid_column_end: grid_column_end.unwrap_or(GridLine::Auto),
        justify_items: justify_items.unwrap_or(initial.justify_items),
        justify_self: justify_self.unwrap_or(initial.justify_self),
        border_collapse: border_collapse.unwrap_or(inherited_border_collapse),
        border_spacing_horizontal: resolved_border_spacing_horizontal,
        border_spacing_vertical: resolved_border_spacing_vertical,
        caption_side: caption_side.unwrap_or(inherited_caption_side),
        table_layout: table_layout.unwrap_or(initial.table_layout),
        empty_cells: empty_cells.unwrap_or(inherited_empty_cells),
        vertical_align: vertical_align
            .map(|v| v.resolve(own_font_size, root_font_size))
            .unwrap_or(initial.vertical_align),
        list_style_type: list_style_type.unwrap_or(inherited_list_style_type),
        list_style_position: list_style_position.unwrap_or(inherited_list_style_position),
        list_style_image: list_style_image.unwrap_or(inherited_list_style_image),
        overflow: overflow.unwrap_or(initial.overflow),
        box_sizing: box_sizing.unwrap_or(initial.box_sizing),
        z_index: z_index.unwrap_or(initial.z_index),
        visibility: visibility.unwrap_or(inherited_visibility),
        outline_width: resolve_len(outline_width, initial.outline_width),
        outline_style: outline_style.unwrap_or(initial.outline_style),
        outline_color: resolved_outline_color,
        quotes: resolved_quotes,
        first_letter_style,
        object_fit: object_fit.unwrap_or(initial.object_fit),
        object_position: resolved_object_position,
        box_shadow: resolved_box_shadow,
        flex_direction: flex_direction.unwrap_or(initial.flex_direction),
        flex_wrap: flex_wrap.unwrap_or(initial.flex_wrap),
        justify_content: justify_content.unwrap_or(initial.justify_content),
        align_items: align_items.unwrap_or(initial.align_items),
        align_content: align_content.unwrap_or(initial.align_content),
        align_self: align_self.unwrap_or(initial.align_self),
        flex_grow: flex_grow.unwrap_or(initial.flex_grow),
        flex_shrink: flex_shrink.unwrap_or(initial.flex_shrink),
        flex_basis: resolved_flex_basis,
        row_gap: resolved_row_gap,
        column_gap: resolved_column_gap,
        transform: resolved_transform,
        transform_origin: resolved_transform_origin,
        opacity: resolved_opacity,
    };

    (style, pushed_counter_names, after_parts)
}

/// `content`パーツの列を実際の文字列へ解決する。`counters`/`quote_depth`は
/// この時点で当該要素自身の`counter-reset`/`counter-increment`適用済みの状態を
/// 渡すこと([0024]決定2)。
fn resolve_content_parts(
    parts: Option<Vec<ContentPart>>,
    dom: &Dom,
    element: NodeId,
    counters: &HashMap<String, Vec<i32>>,
    quote_depth: &mut i32,
    quotes: &Option<Vec<QuotePair>>,
) -> Option<String> {
    let parts = parts?;
    let mut result = String::new();
    for part in parts {
        match part {
            ContentPart::String(s) => result.push_str(&s),
            ContentPart::Attr(name) => {
                if let Some(value) = read_element_attr(dom, element, &name) {
                    result.push_str(&value);
                }
            }
            ContentPart::Counter(name, style) => {
                let value = counters
                    .get(&name)
                    .and_then(|s| s.last())
                    .copied()
                    .unwrap_or(0);
                result.push_str(&format_counter_value(style, value));
            }
            ContentPart::Counters(name, separator, style) => {
                if let Some(stack) = counters.get(&name) {
                    let formatted: Vec<String> = stack
                        .iter()
                        .map(|&v| format_counter_value(style, v))
                        .collect();
                    result.push_str(&formatted.join(&separator));
                }
            }
            ContentPart::OpenQuote => {
                result.push_str(&quote_text(quotes, *quote_depth, true));
                *quote_depth += 1;
            }
            ContentPart::CloseQuote => {
                *quote_depth = (*quote_depth - 1).max(0);
                result.push_str(&quote_text(quotes, *quote_depth, false));
            }
            ContentPart::NoOpenQuote => *quote_depth += 1,
            ContentPart::NoCloseQuote => *quote_depth = (*quote_depth - 1).max(0),
        }
    }
    Some(result)
}

/// `quotes`の`depth`階層目の開き/閉じ引用符。`quotes: none`または未指定は
/// 常に空文字列(決定3)。深度が指定ペア数を超えた場合は最後のペアを繰り返す。
fn quote_text(quotes: &Option<Vec<QuotePair>>, depth: i32, is_open: bool) -> String {
    let Some(pairs) = quotes else {
        return String::new();
    };
    let Some(last_index) = pairs.len().checked_sub(1) else {
        return String::new();
    };
    let index = (depth.max(0) as usize).min(last_index);
    let pair = &pairs[index];
    if is_open {
        pair.open.clone()
    } else {
        pair.close.clone()
    }
}

/// `list-style-type`の値からカウンタ表記(`content: counter()`用)を生成する。
/// `list-style-type`の同名のマーカー生成([`crate::layout::box_tree`])と異なり、
/// 末尾に`.`を付けない。`disc`/`circle`/`square`/`none`はカウンタ表記として
/// 意味を持たないため空文字列を返す(仕様通り)。
pub(crate) fn format_counter_value(style: ListStyleType, n: i32) -> String {
    let n = n.max(0) as usize;
    match style {
        ListStyleType::None
        | ListStyleType::Disc
        | ListStyleType::Circle
        | ListStyleType::Square => String::new(),
        ListStyleType::Decimal => n.to_string(),
        ListStyleType::DecimalLeadingZero => format!("{n:02}"),
        ListStyleType::LowerRoman => crate::numbering::to_roman(n).to_lowercase(),
        ListStyleType::UpperRoman => crate::numbering::to_roman(n),
        ListStyleType::LowerAlpha => crate::numbering::to_alpha(n).to_lowercase(),
        ListStyleType::UpperAlpha => crate::numbering::to_alpha(n),
    }
}

/// margin box(`@top-left`等)の`content`を解決する。本文の`content:
/// counter()`(DOM順カウンタスコープ、`compute_recursive`)とはタイミングが
/// 根本的に異なる(ページ分割**後**)ため、別経路として実装する
/// ([0028](../../../docs/decisions/0028-paged-media-design.md)決定6)。
/// `counter(page)`/`counter(pages)`(`counters()`形式も含む、区切り文字は
/// 意味を持たない)のみ値を持ち、それ以外の名前付きカウンタ・`attr()`・
/// 引用符は常に空文字列になる(margin boxにはDOM要素・カウンタスコープ・
/// 引用符ネスト深度という概念が無いため、既知の簡略化)。
pub fn resolve_margin_box_content(
    parts: &[ContentPart],
    page_number: usize,
    total_pages: Option<usize>,
) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            ContentPart::String(s) => out.push_str(s),
            ContentPart::Counter(name, style) | ContentPart::Counters(name, _, style) => {
                if let Some(n) = page_counter_value(name, page_number, total_pages) {
                    out.push_str(&format_counter_value(*style, n));
                }
            }
            ContentPart::Attr(_)
            | ContentPart::OpenQuote
            | ContentPart::CloseQuote
            | ContentPart::NoOpenQuote
            | ContentPart::NoCloseQuote => {}
        }
    }
    out
}

fn page_counter_value(name: &str, page_number: usize, total_pages: Option<usize>) -> Option<i32> {
    if name == "page" {
        Some(page_number as i32)
    } else if name == "pages" {
        total_pages.map(|n| n as i32)
    } else {
        None
    }
}

/// `element`のHTML属性値を読む(`content: attr(name)`用)。
fn read_element_attr(dom: &Dom, element: NodeId, name: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &dom.node(element).data else {
        return None;
    };
    attrs
        .iter()
        .find(|attr| &*attr.name.local == name)
        .map(|attr| attr.value.to_string())
}

/// `::first-letter`にマッチした宣言列から、対応するプロパティのみを抜き出して
/// [`FirstLetterStyle`]を組み立てる([0024]決定4、フルの`ComputedStyle`解決は
/// 行わない軽量な実装)。`own_font_size`は`em`単位解決の基準(ホスト要素自身の
/// 計算済みfont-size)。
fn compute_first_letter_style(
    declarations: &[&PropertyDeclaration],
    own_font_size: f32,
    root_font_size: f32,
) -> Option<FirstLetterStyle> {
    if declarations.is_empty() {
        return None;
    }
    let mut style = FirstLetterStyle::default();
    let mut any = false;
    for decl in declarations {
        match decl {
            PropertyDeclaration::FontSize(v) => {
                style.font_size = Some(v.resolve(own_font_size, root_font_size));
                any = true;
            }
            PropertyDeclaration::FontFamily(v) => {
                style.font_family = Some(v.clone());
                any = true;
            }
            PropertyDeclaration::FontWeight(v) => {
                style.font_weight = Some(*v);
                any = true;
            }
            PropertyDeclaration::FontStyle(v) => {
                style.font_style = Some(*v);
                any = true;
            }
            // `currentcolor`はホスト要素自身の色をそのまま使うのと実質的に
            // 同じ結果になるため、明示的な解決をせず「未指定」と同一視する
            // (既知の簡略化)。
            PropertyDeclaration::Color(Color::Rgba {
                red,
                green,
                blue,
                alpha,
            }) => {
                style.color = Some(RgbaColor {
                    red: *red,
                    green: *green,
                    blue: *blue,
                    alpha: *alpha,
                });
                any = true;
            }
            PropertyDeclaration::TextDecorationLine(v) => {
                style.text_decoration_line = Some(*v);
                any = true;
            }
            PropertyDeclaration::TextTransform(v) => {
                style.text_transform = Some(*v);
                any = true;
            }
            _ => {}
        }
    }
    any.then_some(style)
}

/// `color`は継承プロパティなので、指定がない場合・`currentcolor`が指定された場合
/// (仕様上は循環するため、継承値をそのまま使う)のいずれも親の計算値を使う。
fn resolve_color(declared: Option<Color>, inherited: RgbaColor) -> RgbaColor {
    match declared {
        Some(Color::Rgba {
            red,
            green,
            blue,
            alpha,
        }) => RgbaColor {
            red,
            green,
            blue,
            alpha,
        },
        Some(Color::CurrentColor) | None => inherited,
    }
}

/// 要素の`style="..."`属性をパースする(属性がなければ空)。
fn inline_style_declarations(dom: &Dom, element: NodeId) -> Vec<PropertyDeclaration> {
    let NodeData::Element { attrs, .. } = &dom.node(element).data else {
        return Vec::new();
    };
    attrs
        .iter()
        .find(|attr| &*attr.name.local == "style")
        .map(|attr| parse_inline_style(&attr.value))
        .unwrap_or_default()
}

/// `data-page-break="before|after|avoid"`属性の糖衣API。対応する`break-before`/
/// `break-after`/`break-inside: avoid`宣言へ変換する(値は大文字小文字を区別しない)。
/// 認識できない値は無視する(通常のCSSの不正値と同様、宣言なしとして扱う)。
fn data_page_break_declarations(dom: &Dom, element: NodeId) -> Vec<PropertyDeclaration> {
    let NodeData::Element { attrs, .. } = &dom.node(element).data else {
        return Vec::new();
    };
    let Some(attr) = attrs
        .iter()
        .find(|attr| &*attr.name.local == "data-page-break")
    else {
        return Vec::new();
    };
    match attr.value.trim().to_ascii_lowercase().as_str() {
        "before" => vec![PropertyDeclaration::BreakBefore(BreakBetween::Always)],
        "after" => vec![PropertyDeclaration::BreakAfter(BreakBetween::Always)],
        "avoid" => vec![PropertyDeclaration::BreakInside(BreakInside::Avoid)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use crate::style::parse_stylesheet;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
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

    #[test]
    fn inherits_color_and_font_family_through_multiple_levels() {
        let dom = html::parse(br#"<div><section><p>text</p></section></div>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { color: rgb(9, 8, 7); font-family: Georgia; }");

        let styles = compute_styles(&dom, &ua, &author);
        let p_style = &styles[&p];

        assert_eq!(
            p_style.color,
            RgbaColor {
                red: 9,
                green: 8,
                blue: 7,
                alpha: 1.0
            }
        );
        assert_eq!(p_style.font_family, vec!["Georgia".to_string()]);
    }

    #[test]
    fn reassigning_inherited_property_stops_old_value_propagation() {
        let dom = html::parse(br#"<div><section><p>text</p></section></div>"#);
        let section = find(&dom, dom.document(), "section").expect("section not found");
        let p = find(&dom, section, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("div { color: rgb(9, 8, 7); } section { color: rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&section].color,
            RgbaColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 1.0
            }
        );
        assert_eq!(
            styles[&p].color,
            RgbaColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn background_color_is_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-color: rgb(5, 5, 5); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 5,
                green: 5,
                blue: 5,
                alpha: 1.0
            }
        );
        assert_eq!(
            styles[&p].background_color,
            ComputedStyle::default().background_color
        );
    }

    #[test]
    fn background_image_is_parsed_and_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(r#"div { background-image: url("bg.png"); }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_image.as_deref(),
            Some("bg.png"),
            "background-image should be parsed and reach ComputedStyle"
        );
        assert_eq!(
            styles[&p].background_image, None,
            "background-image should not be inherited"
        );
    }

    #[test]
    fn background_image_none_overrides_an_earlier_url_in_the_cascade() {
        let dom = html::parse(br#"<div class="a b"></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            r#".a { background-image: url("bg.png"); } .b { background-image: none; }"#,
        );

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_image, None,
            "a later `background-image: none` should win the cascade over an earlier url()"
        );
    }

    #[test]
    fn background_position_keyword_pairs_are_order_independent() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-position: bottom right; }");
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position.horizontal, LengthPercentage::Percentage(1.0));
        assert_eq!(position.vertical, LengthPercentage::Percentage(1.0));

        let author = parse_stylesheet("div { background-position: right bottom; }");
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position.horizontal, LengthPercentage::Percentage(1.0));
        assert_eq!(position.vertical, LengthPercentage::Percentage(1.0));
    }

    #[test]
    fn background_position_single_keyword_centers_the_other_axis() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-position: top; }");
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position.horizontal, LengthPercentage::Percentage(0.5));
        assert_eq!(position.vertical, LengthPercentage::Percentage(0.0));
    }

    #[test]
    fn background_position_mixes_keyword_and_length() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-position: 20px top; }");
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position.horizontal, LengthPercentage::Length(20.0));
        assert_eq!(position.vertical, LengthPercentage::Percentage(0.0));
    }

    #[test]
    fn background_position_rejects_same_axis_keyword_pairs() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // `left right`は両方水平軸のキーワードで無効 → 宣言ごと無視され初期値のまま。
        let author = parse_stylesheet("div { background-position: left right; }");
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position, BackgroundPosition::default());
    }

    #[test]
    fn background_position_default_is_top_left() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let position = styles[&div].background_position;
        assert_eq!(position.horizontal, LengthPercentage::Percentage(0.0));
        assert_eq!(position.vertical, LengthPercentage::Percentage(0.0));
    }

    #[test]
    fn background_size_keywords_and_single_value() {
        let dom =
            html::parse(br#"<div class="a"></div><div class="b"></div><div class="c"></div>"#);
        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let [a, b, c] = divs[..] else {
            panic!("expected exactly 3 divs")
        };

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            r#".a { background-size: cover; }
               .b { background-size: contain; }
               .c { background-size: 50%; }"#,
        );
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&a].background_size, BackgroundSize::Cover);
        assert_eq!(styles[&b].background_size, BackgroundSize::Contain);
        assert_eq!(
            styles[&c].background_size,
            BackgroundSize::WidthHeight(
                LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Percentage(0.5)),
                LengthPercentageOrAuto::Auto
            )
        );
    }

    #[test]
    fn object_fit_keywords_are_parsed_and_default_is_fill() {
        let dom = html::parse(
            br#"<img class="a"><img class="b"><img class="c"><img class="d"><img class="e"><img class="f">"#,
        );
        let mut imgs = Vec::new();
        find_all(&dom, dom.document(), "img", &mut imgs);
        let [a, b, c, d, e, f] = imgs[..] else {
            panic!("expected exactly 6 imgs")
        };

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            r#".a { object-fit: fill; }
               .b { object-fit: contain; }
               .c { object-fit: cover; }
               .d { object-fit: none; }
               .e { object-fit: scale-down; }"#,
        );
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&a].object_fit, ObjectFit::Fill);
        assert_eq!(styles[&b].object_fit, ObjectFit::Contain);
        assert_eq!(styles[&c].object_fit, ObjectFit::Cover);
        assert_eq!(styles[&d].object_fit, ObjectFit::None);
        assert_eq!(styles[&e].object_fit, ObjectFit::ScaleDown);
        // 未指定時の初期値は`fill`。
        assert_eq!(styles[&f].object_fit, ObjectFit::Fill);
    }

    #[test]
    fn object_position_default_is_50_percent_and_can_be_overridden() {
        let dom = html::parse(br#"<img class="a"><img class="b">"#);
        let mut imgs = Vec::new();
        find_all(&dom, dom.document(), "img", &mut imgs);
        let [a, b] = imgs[..] else {
            panic!("expected exactly 2 imgs")
        };

        let ua = Stylesheet::default();
        let author = parse_stylesheet(".b { object-position: right bottom; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&a].object_position,
            BackgroundPosition {
                horizontal: LengthPercentage::Percentage(0.5),
                vertical: LengthPercentage::Percentage(0.5),
            }
        );
        assert_eq!(
            styles[&b].object_position,
            BackgroundPosition {
                horizontal: LengthPercentage::Percentage(1.0),
                vertical: LengthPercentage::Percentage(1.0),
            }
        );
    }

    #[test]
    fn box_shadow_defaults_to_empty_and_parses_offsets_blur_spread() {
        let dom = html::parse(br#"<div class="a"></div><div class="b"></div>"#);
        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let [a, b] = divs[..] else {
            panic!("expected exactly 2 divs")
        };

        let ua = Stylesheet::default();
        let author = parse_stylesheet(".b { box-shadow: 2px 3px 4px 5px rgb(10, 20, 30); }");
        let styles = compute_styles(&dom, &ua, &author);
        assert!(styles[&a].box_shadow.is_empty());

        let shadows = &styles[&b].box_shadow;
        assert_eq!(shadows.len(), 1);
        let shadow = shadows[0];
        assert_eq!(shadow.offset_x, 2.0);
        assert_eq!(shadow.offset_y, 3.0);
        assert_eq!(shadow.blur_radius, 4.0);
        assert_eq!(shadow.spread_radius, 5.0);
        assert_eq!(
            shadow.color,
            RgbaColor {
                red: 10,
                green: 20,
                blue: 30,
                alpha: 1.0
            }
        );
        assert!(!shadow.inset);
    }

    #[test]
    fn box_shadow_supports_comma_separated_list_inset_and_currentcolor() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            "div { color: rgb(9, 8, 7); \
             box-shadow: 1px 1px, inset 2px 2px 3px rgb(1,1,1); }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let shadows = &styles[&div].box_shadow;
        assert_eq!(shadows.len(), 2);
        // 1つ目: 色省略時は`currentcolor`(この要素の計算済み`color`)へ解決される。
        assert_eq!(
            shadows[0].color,
            RgbaColor {
                red: 9,
                green: 8,
                blue: 7,
                alpha: 1.0
            }
        );
        assert_eq!(shadows[0].blur_radius, 0.0);
        assert!(!shadows[0].inset);
        // 2つ目: `inset`はパースされる(描画は非対応、決定1)。
        assert!(shadows[1].inset);
    }

    #[test]
    fn box_shadow_none_clears_the_shorthand() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { box-shadow: none; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert!(styles[&div].box_shadow.is_empty());
    }

    #[test]
    fn background_repeat_and_attachment_are_parsed_and_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("div { background-repeat: repeat-x; background-attachment: fixed; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].background_repeat, BackgroundRepeat::RepeatX);
        assert_eq!(
            styles[&div].background_attachment,
            BackgroundAttachment::Fixed
        );
        assert_eq!(styles[&p].background_repeat, BackgroundRepeat::Repeat);
        assert_eq!(
            styles[&p].background_attachment,
            BackgroundAttachment::Scroll
        );
    }

    #[test]
    fn background_shorthand_resets_unspecified_longhands_to_initial_values() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // 先に個別プロパティで背景画像・repeatを設定した後、`background`
        // ショートハンドが色だけを指定 → 仕様通り他の値は初期値へ戻るはず。
        let author = parse_stylesheet(
            r#"div { background-image: url("bg.png"); background-repeat: no-repeat; }
               div { background: red; }"#,
        );
        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(
            style.background_color,
            RgbaColor {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 1.0
            }
        );
        assert_eq!(
            style.background_image, None,
            "background shorthand should reset background-image to none"
        );
        assert_eq!(
            style.background_repeat,
            BackgroundRepeat::Repeat,
            "background shorthand should reset background-repeat to its initial value"
        );
    }

    #[test]
    fn background_shorthand_parses_position_and_size_with_slash() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet(r#"div { background: url("bg.png") no-repeat center / cover; }"#);
        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.background_image.as_deref(), Some("bg.png"));
        assert_eq!(style.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            style.background_position.horizontal,
            LengthPercentage::Percentage(0.5)
        );
        assert_eq!(style.background_size, BackgroundSize::Cover);
    }

    #[test]
    fn hsl_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // 純粋な赤(hue=0, saturation=100%, lightness=50%) = rgb(255, 0, 0)。
        let author = parse_stylesheet("div { color: hsl(0deg 100% 50%); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].color,
            RgbaColor {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn hwb_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // 白100% -> 完全な白 rgb(255, 255, 255)。
        let author = parse_stylesheet("div { color: hwb(0deg 100% 0%); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].color,
            RgbaColor {
                red: 255,
                green: 255,
                blue: 255,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn hsl_color_function_with_alpha_is_preserved() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { background-color: hsl(0deg 0% 0% / 50%); }");

        let styles = compute_styles(&dom, &ua, &author);
        let bg = styles[&div].background_color;
        assert_eq!((bg.red, bg.green, bg.blue), (0, 0, 0));
        assert!((bg.alpha - 0.5).abs() < 0.01);
    }

    #[test]
    fn lab_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // lab(53.2408% 80.0925 67.2032) は純粋な赤 rgb(255, 0, 0) に相当する。
        let author = parse_stylesheet("div { color: lab(53.2408% 80.0925 67.2032); }");

        let styles = compute_styles(&dom, &ua, &author);
        let color = styles[&div].color;
        assert_eq!(color.red, 255);
        assert!(color.green <= 1);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn lch_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // lch(53.2408% 104.5518 39.999deg) は純粋な赤 rgb(255, 0, 0) に相当する。
        let author = parse_stylesheet("div { color: lch(53.2408% 104.5518 39.999deg); }");

        let styles = compute_styles(&dom, &ua, &author);
        let color = styles[&div].color;
        assert_eq!(color.red, 255);
        assert!(color.green <= 1);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn oklab_color_function_resolves_to_expected_rgb() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // oklab(62.8% 0.2249 0.1258) は純粋な赤 rgb(255, 0, 0) に相当する。
        let author = parse_stylesheet("div { color: oklab(62.8% 0.2249 0.1258); }");

        let styles = compute_styles(&dom, &ua, &author);
        let color = styles[&div].color;
        assert_eq!(color.red, 255);
        assert!(color.green <= 1);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn oklch_color_function_with_alpha_is_preserved() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // oklch(59.686% 0.15619 49.7694deg)は#ba5d06相当(rgb(198, 93, 6))。
        let author =
            parse_stylesheet("div { background-color: oklch(59.686% 0.15619 49.7694deg / 50%); }");

        let styles = compute_styles(&dom, &ua, &author);
        let bg = styles[&div].background_color;
        assert_eq!((bg.red, bg.green, bg.blue), (198, 93, 6));
        assert!((bg.alpha - 0.5).abs() < 0.01);
    }

    #[test]
    fn current_color_background_resolves_to_own_computed_color() {
        let dom = html::parse(br#"<div>text</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("div { color: rgb(4, 5, 6); background-color: currentcolor; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 4,
                green: 5,
                blue: 6,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn root_without_declarations_gets_initial_values() {
        let dom = html::parse(br#"<div>text</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        let default = ComputedStyle::default();
        assert_eq!(styles[&div].color, default.color);
        assert_eq!(styles[&div].font_size, default.font_size);
        assert_eq!(styles[&div].font_family, default.font_family);
    }

    #[test]
    fn non_element_nodes_inherit_parent_style_directly() {
        let dom = html::parse(br#"<p>hello</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let text = dom.children(p).next().expect("text node not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("p { color: rgb(7, 7, 7); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&text], styles[&p]);
    }

    #[test]
    fn inline_style_overrides_stylesheet_rules_regardless_of_specificity() {
        // #idセレクタは通常どのクラス/type選択子よりも詳細度が高いが、
        // インラインstyleはそれよりもさらに優先されるはず。
        let dom = html::parse(br#"<div id="x" style="color: rgb(9, 9, 9);">t</div>"#);
        let p = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("#x { color: rgb(1, 1, 1); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&p].color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn inline_style_applies_when_there_is_no_matching_rule() {
        let dom = html::parse(br#"<div style="background-color: rgb(4, 5, 6);">t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].background_color,
            RgbaColor {
                red: 4,
                green: 5,
                blue: 6,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn font_weight_and_style_are_inherited_but_overridable() {
        let dom = html::parse(br#"<p><b>bold <i>bold-italic</i></b></p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let b = find(&dom, p, "b").expect("b not found");
        let i = find(&dom, b, "i").expect("i not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("b { font-weight: bold; } i { font-style: italic; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&p].font_weight, super::FontWeight::Normal);
        assert_eq!(styles[&b].font_weight, super::FontWeight::Bold);
        assert_eq!(styles[&b].font_style, super::FontStyle::Normal);
        // <i>は<b>からfont-weight: boldを継承しつつ、自身のfont-style: italicを追加する。
        assert_eq!(styles[&i].font_weight, super::FontWeight::Bold);
        assert_eq!(styles[&i].font_style, super::FontStyle::Italic);
    }

    #[test]
    fn text_decoration_line_parses_underline_and_line_through() {
        let dom = html::parse(br#"<p>a</p>"#);

        let underline = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration: underline; }"),
        );
        let p = find(&dom, dom.document(), "p").expect("p not found");
        assert!(underline[&p].text_decoration_line.underline);
        assert!(!underline[&p].text_decoration_line.line_through);

        let line_through = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration-line: line-through; }"),
        );
        assert!(line_through[&p].text_decoration_line.line_through);
        assert!(!line_through[&p].text_decoration_line.underline);

        let both = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-decoration: underline line-through; }"),
        );
        assert!(both[&p].text_decoration_line.underline);
        assert!(both[&p].text_decoration_line.line_through);
    }

    #[test]
    fn text_decoration_line_propagates_to_descendants_like_font_weight() {
        // 仕様上は非継承だが、祖先の装飾線が子孫へ伝播する特殊規則の代わりに
        // このリポジトリでは継承として扱う簡略実装(computed.rsのコメント参照)。
        let dom = html::parse(br#"<u>bold <b>text</b></u>"#);
        let u = find(&dom, dom.document(), "u").expect("u not found");
        let b = find(&dom, u, "b").expect("b not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("u { text-decoration: underline; }"),
        );
        assert!(styles[&u].text_decoration_line.underline);
        assert!(styles[&b].text_decoration_line.underline);
    }

    #[test]
    fn ua_stylesheet_gives_u_and_s_their_default_text_decoration() {
        use super::super::ua::user_agent_stylesheet;

        let dom = html::parse(br#"<p><u>underlined</u> <s>struck</s></p>"#);
        let u = find(&dom, dom.document(), "u").expect("u not found");
        let s = find(&dom, dom.document(), "s").expect("s not found");

        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());
        assert!(styles[&u].text_decoration_line.underline);
        assert!(styles[&s].text_decoration_line.line_through);
    }

    #[test]
    fn ua_stylesheet_gives_pre_its_default_white_space() {
        use super::super::ua::user_agent_stylesheet;

        let dom = html::parse(br#"<pre>  a   b  </pre>"#);
        let pre = find(&dom, dom.document(), "pre").expect("pre not found");

        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());
        assert_eq!(styles[&pre].white_space, super::WhiteSpace::Pre);
    }

    #[test]
    fn text_decoration_none_overrides_inherited_underline() {
        let dom = html::parse(br#"<u><span class="plain">text</span></u>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet("u { text-decoration: underline; } .plain { text-decoration: none; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert!(!styles[&span].text_decoration_line.underline);
    }

    #[test]
    fn numeric_font_weight_is_thresholded_to_bold_or_normal() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let light = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { font-weight: 400; }"),
        );
        assert_eq!(light[&p].font_weight, super::FontWeight::Normal);

        let heavy = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { font-weight: 700; }"),
        );
        assert_eq!(heavy[&p].font_weight, super::FontWeight::Bold);
    }

    #[test]
    fn elements_without_style_attribute_are_unaffected() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div], ComputedStyle::default());
    }

    #[test]
    fn border_shorthand_sets_width_style_and_color_on_all_sides() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border: 2px dashed rgb(10, 20, 30); }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_top_width.0, 2.0);
        assert_eq!(style.border_right_width.0, 2.0);
        assert_eq!(style.border_bottom_width.0, 2.0);
        assert_eq!(style.border_left_width.0, 2.0);
        assert_eq!(style.border_top_style, super::BorderStyle::Dashed);
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 10,
                green: 20,
                blue: 30,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn border_top_width_longhand_sets_only_the_top_edge() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border-top-width: 5px; }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_top_width.0, 5.0);
        assert_eq!(style.border_right_width.0, 0.0);
        assert_eq!(style.border_bottom_width.0, 0.0);
        assert_eq!(style.border_left_width.0, 0.0);
    }

    #[test]
    fn border_edge_longhands_set_width_style_and_color_independently() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            "div { border-bottom-width: 3px; border-bottom-style: dotted; \
             border-bottom-color: rgb(1, 2, 3); }",
        );

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_bottom_width.0, 3.0);
        assert_eq!(style.border_bottom_style, super::BorderStyle::Dotted);
        assert_eq!(
            style.border_bottom_color,
            RgbaColor {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 1.0
            }
        );
        // 他の辺には影響しない。
        assert_eq!(style.border_top_width.0, 0.0);
        assert_eq!(style.border_top_style, super::BorderStyle::None);
    }

    #[test]
    fn border_edge_shorthand_sets_width_style_and_color_on_one_side() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border-left: 4px solid rgb(5, 6, 7); }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_left_width.0, 4.0);
        assert_eq!(style.border_left_style, super::BorderStyle::Solid);
        assert_eq!(
            style.border_left_color,
            RgbaColor {
                red: 5,
                green: 6,
                blue: 7,
                alpha: 1.0
            }
        );
        // 他の辺には影響しない(初期値のまま)。
        assert_eq!(style.border_right_width.0, 0.0);
        assert_eq!(style.border_right_style, super::BorderStyle::None);
    }

    #[test]
    fn opacity_parses_and_clamps_out_of_range_values() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { opacity: 0.5; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].opacity, 0.5);

        let author = parse_stylesheet("div { opacity: 2; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].opacity, 1.0);

        let author = parse_stylesheet("div { opacity: -1; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].opacity, 0.0);
    }

    #[test]
    fn opacity_defaults_to_one_and_is_not_inherited() {
        let dom = html::parse(br#"<div><p>t</p></div>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { opacity: 0.3; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&p].opacity, 1.0);
    }

    #[test]
    fn transform_parses_multiple_functions_and_none_resets_it() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { transform: translateX(10px) scale(2); }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].transform.len(), 2);

        let author = parse_stylesheet("div { transform: none; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert!(styles[&div].transform.is_empty());
    }

    #[test]
    fn transform_origin_defaults_to_50_percent_and_can_be_overridden() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].transform_origin.horizontal,
            LengthPercentage::Percentage(0.5)
        );
        assert_eq!(
            styles[&div].transform_origin.vertical,
            LengthPercentage::Percentage(0.5)
        );

        let author = parse_stylesheet("div { transform-origin: left top; }");
        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&div].transform_origin.horizontal,
            LengthPercentage::Percentage(0.0)
        );
        assert_eq!(
            styles[&div].transform_origin.vertical,
            LengthPercentage::Percentage(0.0)
        );
    }

    #[test]
    fn border_color_defaults_to_currentcolor_when_unspecified() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { color: rgb(9, 9, 9); border: 1px solid; }");

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            },
            "border-color should follow currentcolor when not explicitly set"
        );
    }

    #[test]
    fn border_color_and_border_style_shorthands_expand_per_side() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(
            "div { border-style: solid dotted; border-color: rgb(1,1,1) rgb(2,2,2); }",
        );

        let styles = compute_styles(&dom, &ua, &author);
        let style = &styles[&div];
        assert_eq!(style.border_top_style, super::BorderStyle::Solid);
        assert_eq!(style.border_right_style, super::BorderStyle::Dotted);
        assert_eq!(style.border_bottom_style, super::BorderStyle::Solid);
        assert_eq!(style.border_left_style, super::BorderStyle::Dotted);
        assert_eq!(
            style.border_top_color,
            RgbaColor {
                red: 1,
                green: 1,
                blue: 1,
                alpha: 1.0
            }
        );
        assert_eq!(
            style.border_right_color,
            RgbaColor {
                red: 2,
                green: 2,
                blue: 2,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn em_font_size_resolves_against_parent_font_size() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        // div: 20px、p: divの1.5倍 = 30px。
        let author = parse_stylesheet("div { font-size: 20px; } p { font-size: 1.5em; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].font_size.0, 20.0);
        assert_eq!(styles[&p].font_size.0, 30.0);
    }

    #[test]
    fn em_length_on_non_font_size_property_uses_own_font_size() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = Stylesheet::default();
        // font-sizeが先に20pxへ解決され、border-widthの2emはそれを基準にする = 40px。
        let author = parse_stylesheet("div { font-size: 20px; border: 2em solid black; }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&div].border_top_width.0, 40.0);
    }

    #[test]
    fn rem_length_resolves_against_root_element_font_size_regardless_of_nesting() {
        let dom = html::parse(br#"<html><body><div><p>text</p></div></body></html>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let ua = Stylesheet::default();
        // ルート(<html>)のfont-sizeを10pxにし、ネストしたpのmargin: 2remが
        // 親(div/body)のfont-sizeに影響されず常に20pxになることを確認する。
        let author = parse_stylesheet(
            "html { font-size: 10px; } div { font-size: 30px; } p { margin: 2rem; }",
        );

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(
            styles[&p].margin_top,
            LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(20.0))
        );
    }

    #[test]
    fn border_is_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet("div { border: 3px solid rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&p].border_top_style, super::BorderStyle::None);
        assert_eq!(styles[&p].border_top_width.0, 0.0);
    }

    #[test]
    fn break_before_and_break_after_default_to_auto() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].break_after, BreakBetween::Auto);
        assert_eq!(styles[&p].break_inside, BreakInside::Auto);
    }

    #[test]
    fn break_before_and_break_after_parse_avoid_and_always() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: avoid; break-after: always; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Avoid);
        assert_eq!(styles[&p].break_after, BreakBetween::Always);
    }

    #[test]
    fn break_before_page_keyword_is_treated_as_always() {
        // 単一ページサイズしか扱わないため、`page`は`always`と同じ効果として扱う。
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: page; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Always);
    }

    #[test]
    fn break_inside_parses_avoid() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-inside: avoid; }"),
        );
        assert_eq!(styles[&p].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn legacy_page_break_properties_are_aliases_for_break_properties() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "p { page-break-before: always; page-break-after: avoid; \
                 page-break-inside: avoid; }",
            ),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Always);
        assert_eq!(styles[&p].break_after, BreakBetween::Avoid);
        assert_eq!(styles[&p].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn orphans_and_widows_default_to_two_and_can_be_overridden() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&p].orphans, 2);
        assert_eq!(defaults[&p].widows, 2);

        let overridden = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { orphans: 3; widows: 4; }"),
        );
        assert_eq!(overridden[&p].orphans, 3);
        assert_eq!(overridden[&p].widows, 4);
    }

    #[test]
    fn orphans_rejects_non_positive_values() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        // 無効な値は宣言ごと無視され、初期値のままになる。
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { orphans: 0; }"),
        );
        assert_eq!(styles[&p].orphans, 2);
    }

    #[test]
    fn break_properties_are_not_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { break-before: always; orphans: 5; }"),
        );
        assert_eq!(styles[&div].break_before, BreakBetween::Always);
        assert_eq!(styles[&div].orphans, 5);
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].orphans, 2);
    }

    #[test]
    fn data_page_break_attribute_maps_to_break_properties() {
        let dom = html::parse(
            br#"<div><p id="a" data-page-break="before">a</p>
                <p id="b" data-page-break="after">b</p>
                <p id="c" data-page-break="avoid">c</p></div>"#,
        );
        let a = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&a].break_before, BreakBetween::Always);

        let mut ps = Vec::new();
        fn find_all(dom: &Dom, id: NodeId, out: &mut Vec<NodeId>) {
            if let NodeData::Element { name, .. } = &dom.node(id).data {
                if &*name.local == "p" {
                    out.push(id);
                }
            }
            for child in dom.children(id) {
                find_all(dom, child, out);
            }
        }
        find_all(&dom, dom.document(), &mut ps);
        assert_eq!(styles[&ps[1]].break_after, BreakBetween::Always);
        assert_eq!(styles[&ps[2]].break_inside, BreakInside::Avoid);
    }

    #[test]
    fn data_page_break_ignores_unrecognized_values() {
        let dom = html::parse(br#"<p data-page-break="sideways">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
        assert_eq!(styles[&p].break_after, BreakBetween::Auto);
        assert_eq!(styles[&p].break_inside, BreakInside::Auto);
    }

    #[test]
    fn stylesheet_rule_overrides_data_page_break_attribute() {
        // 属性糖衣は「スタイルシートで個別に上書きできる既定のヒント」という
        // 位置づけなので、通常のCSSルールの方が優先される。
        let dom = html::parse(br#"<p data-page-break="before">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { break-before: auto; }"),
        );
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
    }

    #[test]
    fn inline_style_overrides_data_page_break_attribute() {
        let dom = html::parse(br#"<p data-page-break="before" style="break-before: auto;">a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].break_before, BreakBetween::Auto);
    }

    #[test]
    fn before_and_after_pseudo_content_resolve_from_matching_rules() {
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author =
            parse_stylesheet(r#".badge::before { content: "["; } .badge::after { content: "]"; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content.as_deref(), Some("["));
        assert_eq!(styles[&span].pseudo_after_content.as_deref(), Some("]"));
    }

    #[test]
    fn pseudo_content_is_none_without_a_matching_before_after_rule() {
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(".badge { color: rgb(1, 2, 3); }");

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
        assert_eq!(styles[&span].pseudo_after_content, None);
    }

    #[test]
    fn explicit_content_none_wins_over_an_earlier_lower_specificity_rule() {
        let dom = html::parse(br#"<span id="x" class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        // クラスセレクタで文字列を指定していても、詳細度の高い#idセレクタが
        // 後から`content: none`にすれば生成ボックスは無くなるはず。
        let author =
            parse_stylesheet(r#".badge::before { content: "x"; } #x::before { content: none; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
    }

    #[test]
    fn float_left_and_right_parse_and_are_not_inherited() {
        let dom = html::parse(br#"<div><img></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let img = find(&dom, div, "img").expect("img not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { float: left; } img { float: right; }"),
        );
        assert_eq!(styles[&div].float, super::Float::Left);
        assert_eq!(styles[&img].float, super::Float::Right);
    }

    #[test]
    fn float_forces_inline_display_to_block() {
        let dom = html::parse(br#"<span>text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("span { float: left; }"),
        );
        assert_eq!(
            styles[&span].display,
            Display::Block,
            "CSS2.1 9.7: floatが指定された要素は自動的にblock-levelになる"
        );
    }

    #[test]
    fn float_none_does_not_affect_display() {
        let dom = html::parse(br#"<span>text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&span].display, Display::Inline);
    }

    #[test]
    fn clear_parses_all_keywords() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("left", super::Clear::Left),
            ("right", super::Clear::Right),
            ("both", super::Clear::Both),
            ("none", super::Clear::None),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ clear: {value}; }}")),
            );
            assert_eq!(styles[&div].clear, expected, "clear: {value}");
        }
    }

    #[test]
    fn position_relative_parses_with_offsets() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { position: relative; top: 5px; left: 10px; }"),
        );
        assert_eq!(styles[&div].position, super::Position::Relative);
        assert_eq!(
            styles[&div].top,
            LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(5.0))
        );
        assert_eq!(
            styles[&div].left,
            LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(10.0))
        );
    }

    #[test]
    fn calc_mixes_percentage_and_pixels() {
        use crate::style::LengthPercentage;
        let dom = html::parse(br#"<div>x</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { width: calc(100% - 40px); }"),
        );
        match styles[&div].width {
            super::LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc {
                px,
                percent,
            }) => {
                assert_eq!(px, -40.0);
                assert_eq!(percent, 1.0);
            }
            other => panic!("expected a calc value, got {other:?}"),
        }
    }

    #[test]
    fn calc_resolves_em_using_the_element_font_size() {
        use crate::style::LengthPercentage;
        let dom = html::parse(br#"<div>x</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { font-size: 20px; margin-left: calc(2em + 5px); }"),
        );
        // 2em(=40px)+ 5px = 45px、パーセンテージ成分なし。
        match styles[&div].margin_left {
            super::LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc {
                px,
                percent,
            }) => {
                assert_eq!(px, 45.0);
                assert_eq!(percent, 0.0);
            }
            other => panic!("expected a calc value, got {other:?}"),
        }
    }

    #[test]
    fn calc_supports_multiplication_and_division() {
        use crate::style::LengthPercentage;
        let dom = html::parse(br#"<div>x</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { width: calc((100% - 20px) / 2 + 3px * 2); }"),
        );
        // (100% - 20px)/2 = 50% - 10px、+ 6px = 50% - 4px。
        match styles[&div].width {
            super::LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc {
                px,
                percent,
            }) => {
                assert!((px - (-4.0)).abs() < 0.001, "px={px}");
                assert!((percent - 0.5).abs() < 0.001, "percent={percent}");
            }
            other => panic!("expected a calc value, got {other:?}"),
        }
    }

    #[test]
    fn calc_with_a_bare_number_or_dimension_product_is_rejected() {
        let dom = html::parse(br#"<div>x</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        // `calc(2)`は裸の数値、`calc(2px * 3px)`は次元×次元でどちらも無効。
        for css in ["div { width: calc(2); }", "div { width: calc(2px * 3px); }"] {
            let styles = compute_styles(&dom, &Stylesheet::default(), &parse_stylesheet(css));
            assert_eq!(
                styles[&div].width,
                super::LengthPercentageOrAuto::Auto,
                "invalid calc should be dropped, leaving the initial value: {css}"
            );
        }
    }

    #[test]
    fn absolute_and_fixed_are_block_level() {
        // CSS2.1 9.7 / [0049]決定2-0: absolute/fixedはdisplayをblock化する。
        // これによりインライン要素(span)も絶対配置の対象になる。
        let dom = html::parse(br#"<span>x</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");
        for value in ["absolute", "fixed"] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("span {{ position: {value}; }}")),
            );
            assert_eq!(
                styles[&span].display,
                super::Display::Block,
                "position: {value} should block-ify an inline element"
            );
        }
    }

    #[test]
    fn position_absolute_and_fixed_parse() {
        // M11 T270([0049])で対応。以前は非対応で`Static`に落としていた。
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("absolute", super::Position::Absolute),
            ("fixed", super::Position::Fixed),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ position: {value}; }}")),
            );
            assert_eq!(styles[&div].position, expected);
        }
    }

    #[test]
    fn top_right_bottom_left_default_to_auto() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&div].top, LengthPercentageOrAuto::Auto);
        assert_eq!(styles[&div].right, LengthPercentageOrAuto::Auto);
        assert_eq!(styles[&div].bottom, LengthPercentageOrAuto::Auto);
        assert_eq!(styles[&div].left, LengthPercentageOrAuto::Auto);
    }

    #[test]
    fn typography_properties_parse_and_are_inherited() {
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { text-align: center; white-space: nowrap; \
                 letter-spacing: 2px; word-spacing: 3px; text-transform: uppercase; }",
            ),
        );
        for id in [div, p] {
            assert_eq!(styles[&id].text_align, super::TextAlign::Center);
            assert_eq!(styles[&id].white_space, super::WhiteSpace::Nowrap);
            assert_eq!(styles[&id].letter_spacing, 2.0);
            assert_eq!(styles[&id].word_spacing, 3.0);
            assert_eq!(styles[&id].text_transform, super::TextTransform::Uppercase);
        }
    }

    #[test]
    fn text_align_parses_all_keywords() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("left", super::TextAlign::Left),
            ("right", super::TextAlign::Right),
            ("center", super::TextAlign::Center),
            ("justify", super::TextAlign::Justify),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ text-align: {value}; }}")),
            );
            assert_eq!(styles[&div].text_align, expected, "text-align: {value}");
        }
    }

    #[test]
    fn line_height_number_and_percentage_are_inherited_unmultiplied() {
        // CSS2.1 10.8.1: <number>/<percentage>の計算値は指定値そのもの
        // (親のfont-sizeで先に乗算した絶対値ではない)。子が異なるfont-sizeを
        // 持っていても、継承される`LineHeight::Number`の値自体は変わらないはず。
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { line-height: 1.5; } p { font-size: 30px; }"),
        );
        assert_eq!(styles[&div].line_height, super::LineHeight::Number(1.5));
        assert_eq!(
            styles[&p].line_height,
            super::LineHeight::Number(1.5),
            "line-height: <number> should be inherited unmultiplied"
        );

        let percentage_styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { line-height: 150%; }"),
        );
        assert_eq!(
            percentage_styles[&div].line_height,
            super::LineHeight::Number(1.5),
            "150% should normalize to the same representation as <number> 1.5"
        );
    }

    #[test]
    fn line_height_length_resolves_to_absolute_px() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { line-height: 24px; }"),
        );
        assert_eq!(styles[&div].line_height, super::LineHeight::Length(24.0));
    }

    #[test]
    fn line_height_defaults_to_normal() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].line_height, super::LineHeight::Normal);
    }

    #[test]
    fn text_indent_percentage_stays_as_a_fraction_until_used() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { text-indent: 10%; }"),
        );
        assert_eq!(
            styles[&p].text_indent,
            LengthPercentage::Percentage(0.1),
            "text-indent percentage should remain unresolved (fraction) at computed-value time"
        );
    }

    #[test]
    fn text_indent_length_and_inheritance() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { text-indent: 20px; }"),
        );
        assert_eq!(styles[&div].text_indent, LengthPercentage::Length(20.0));
        assert_eq!(
            styles[&p].text_indent,
            LengthPercentage::Length(20.0),
            "text-indent should be inherited"
        );
    }

    #[test]
    fn white_space_parses_all_keywords() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("normal", super::WhiteSpace::Normal),
            ("nowrap", super::WhiteSpace::Nowrap),
            ("pre", super::WhiteSpace::Pre),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ white-space: {value}; }}")),
            );
            assert_eq!(styles[&div].white_space, expected, "white-space: {value}");
        }
    }

    #[test]
    fn letter_spacing_and_word_spacing_default_to_zero_and_resolve_em() {
        let dom = html::parse(br#"<p>a</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&p].letter_spacing, 0.0);
        assert_eq!(defaults[&p].word_spacing, 0.0);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("p { font-size: 20px; letter-spacing: 0.5em; }"),
        );
        assert_eq!(styles[&p].letter_spacing, 10.0);
    }

    #[test]
    fn text_transform_parses_all_keywords() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("none", super::TextTransform::None),
            ("uppercase", super::TextTransform::Uppercase),
            ("lowercase", super::TextTransform::Lowercase),
            ("capitalize", super::TextTransform::Capitalize),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ text-transform: {value}; }}")),
            );
            assert_eq!(
                styles[&div].text_transform, expected,
                "text-transform: {value}"
            );
        }
    }

    #[test]
    fn table_layout_properties_parse_and_have_correct_inheritance() {
        let dom = html::parse(br#"<table><tr><td>a</td></tr></table>"#);
        let table = find(&dom, dom.document(), "table").expect("table not found");
        let td = find(&dom, table, "td").expect("td not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "table { border-collapse: collapse; border-spacing: 3px 5px; \
                 caption-side: bottom; empty-cells: hide; table-layout: fixed; \
                 vertical-align: middle; }",
            ),
        );
        let table_style = &styles[&table];
        assert_eq!(table_style.border_collapse, super::BorderCollapse::Collapse);
        assert_eq!(table_style.border_spacing_horizontal.0, 3.0);
        assert_eq!(table_style.border_spacing_vertical.0, 5.0);
        assert_eq!(table_style.caption_side, super::CaptionSide::Bottom);
        assert_eq!(table_style.empty_cells, super::EmptyCells::Hide);
        assert_eq!(table_style.table_layout, super::TableLayout::Fixed);
        assert_eq!(table_style.vertical_align, super::VerticalAlign::Middle);

        let td_style = &styles[&td];
        // 継承プロパティ: border-collapse/border-spacing/caption-side/empty-cells。
        assert_eq!(td_style.border_collapse, super::BorderCollapse::Collapse);
        assert_eq!(td_style.border_spacing_horizontal.0, 3.0);
        assert_eq!(td_style.caption_side, super::CaptionSide::Bottom);
        assert_eq!(td_style.empty_cells, super::EmptyCells::Hide);
        // 非継承プロパティ: table-layout/vertical-align(tdは初期値のまま)。
        assert_eq!(td_style.table_layout, super::TableLayout::Auto);
        assert_eq!(td_style.vertical_align, super::VerticalAlign::Baseline);
    }

    #[test]
    fn border_spacing_single_value_applies_to_both_axes() {
        let dom = html::parse(br#"<table></table>"#);
        let table = find(&dom, dom.document(), "table").expect("table not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("table { border-spacing: 4px; }"),
        );
        assert_eq!(styles[&table].border_spacing_horizontal.0, 4.0);
        assert_eq!(styles[&table].border_spacing_vertical.0, 4.0);
    }

    #[test]
    fn table_layout_properties_default_correctly() {
        let dom = html::parse(br#"<table><tr><td>a</td></tr></table>"#);
        let table = find(&dom, dom.document(), "table").expect("table not found");

        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        let style = &styles[&table];
        assert_eq!(style.border_collapse, super::BorderCollapse::Separate);
        assert_eq!(style.border_spacing_horizontal.0, 0.0);
        assert_eq!(style.border_spacing_vertical.0, 0.0);
        assert_eq!(style.caption_side, super::CaptionSide::Top);
        assert_eq!(style.table_layout, super::TableLayout::Auto);
        assert_eq!(style.empty_cells, super::EmptyCells::Show);
        assert_eq!(style.vertical_align, super::VerticalAlign::Baseline);
    }

    #[test]
    fn caption_element_gets_table_caption_display_from_ua_stylesheet() {
        use super::super::ua::user_agent_stylesheet;

        let dom = html::parse(br#"<table><caption>Title</caption></table>"#);
        let caption = find(&dom, dom.document(), "caption").expect("caption not found");

        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());
        assert_eq!(styles[&caption].display, Display::TableCaption);
    }

    #[test]
    fn pseudo_content_ignores_declarations_on_the_real_element() {
        // `::before`/`::after`を伴わない通常のセレクタでの`content`宣言は無効。
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let ua = Stylesheet::default();
        let author = parse_stylesheet(r#".badge { content: "x"; }"#);

        let styles = compute_styles(&dom, &ua, &author);
        assert_eq!(styles[&span].pseudo_before_content, None);
        assert_eq!(styles[&span].pseudo_after_content, None);
    }

    #[test]
    fn list_style_properties_default_to_disc_outside_and_no_image() {
        let dom = html::parse(br#"<li>a</li>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");
        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&li].list_style_type, super::ListStyleType::Disc);
        assert_eq!(
            styles[&li].list_style_position,
            super::ListStylePosition::Outside
        );
        assert_eq!(styles[&li].list_style_image, None);
    }

    #[test]
    fn list_style_type_parses_all_keywords() {
        let dom = html::parse(br#"<li>a</li>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");

        for (value, expected) in [
            ("disc", super::ListStyleType::Disc),
            ("circle", super::ListStyleType::Circle),
            ("square", super::ListStyleType::Square),
            ("decimal", super::ListStyleType::Decimal),
            (
                "decimal-leading-zero",
                super::ListStyleType::DecimalLeadingZero,
            ),
            ("lower-roman", super::ListStyleType::LowerRoman),
            ("upper-roman", super::ListStyleType::UpperRoman),
            ("lower-alpha", super::ListStyleType::LowerAlpha),
            ("upper-alpha", super::ListStyleType::UpperAlpha),
            ("none", super::ListStyleType::None),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("li {{ list-style-type: {value}; }}")),
            );
            assert_eq!(
                styles[&li].list_style_type, expected,
                "list-style-type: {value}"
            );
        }
    }

    #[test]
    fn list_style_properties_are_inherited() {
        let dom = html::parse(br#"<ul><li>a</li></ul>"#);
        let ul = find(&dom, dom.document(), "ul").expect("ul not found");
        let li = find(&dom, ul, "li").expect("li not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "ul { list-style-type: square; list-style-position: inside; \
                 list-style-image: url(marker.png); }",
            ),
        );
        assert_eq!(styles[&li].list_style_type, super::ListStyleType::Square);
        assert_eq!(
            styles[&li].list_style_position,
            super::ListStylePosition::Inside
        );
        assert_eq!(styles[&li].list_style_image.as_deref(), Some("marker.png"));
    }

    #[test]
    fn list_style_shorthand_expands_to_all_three_longhands() {
        let dom = html::parse(br#"<li>a</li>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("li { list-style: square inside url(marker.png); }"),
        );
        assert_eq!(styles[&li].list_style_type, super::ListStyleType::Square);
        assert_eq!(
            styles[&li].list_style_position,
            super::ListStylePosition::Inside
        );
        assert_eq!(styles[&li].list_style_image.as_deref(), Some("marker.png"));
    }

    #[test]
    fn list_style_shorthand_none_clears_type_and_image() {
        let dom = html::parse(br#"<li>a</li>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("li { list-style: none; }"),
        );
        assert_eq!(styles[&li].list_style_type, super::ListStyleType::None);
        assert_eq!(styles[&li].list_style_image, None);
    }

    #[test]
    fn list_style_shorthand_type_then_bare_none_means_image_none() {
        // `type`が先に確定した後に出てくる`none`は`list-style-image: none`と
        // 解釈されるべき(`list-style-type`を上書きしない)。
        let dom = html::parse(br#"<li>a</li>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("li { list-style: square none; }"),
        );
        assert_eq!(styles[&li].list_style_type, super::ListStyleType::Square);
        assert_eq!(styles[&li].list_style_image, None);
    }

    #[test]
    fn li_gets_list_item_display_from_ua_stylesheet() {
        use super::super::ua::user_agent_stylesheet;

        let dom = html::parse(br#"<ul><li>a</li></ul>"#);
        let li = find(&dom, dom.document(), "li").expect("li not found");

        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());
        assert_eq!(styles[&li].display, Display::ListItem);
    }

    #[test]
    fn padding_left_and_margin_left_longhands_parse_directly() {
        // ショートハンド(`padding`/`margin`)を経由しない単独のロングハンドの
        // パース([0022]実装中に発見・修正したギャップの回帰テスト)。
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { padding-left: 12px; padding-top: 3px; \
                 margin-left: 5px; margin-top: 7px; }",
            ),
        );
        assert_eq!(
            styles[&div].padding_left,
            super::LengthPercentage::Length(12.0)
        );
        assert_eq!(
            styles[&div].padding_top,
            super::LengthPercentage::Length(3.0)
        );
        assert_eq!(
            styles[&div].margin_left,
            super::LengthPercentageOrAuto::LengthPercentage(super::LengthPercentage::Length(5.0))
        );
        assert_eq!(
            styles[&div].margin_top,
            super::LengthPercentageOrAuto::LengthPercentage(super::LengthPercentage::Length(7.0))
        );
    }

    #[test]
    fn overflow_parses_all_keywords_and_defaults_to_visible() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&div].overflow, super::Overflow::Visible);

        for (value, expected) in [
            ("visible", super::Overflow::Visible),
            ("hidden", super::Overflow::Hidden),
            ("scroll", super::Overflow::Scroll),
            ("auto", super::Overflow::Auto),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ overflow: {value}; }}")),
            );
            assert_eq!(styles[&div].overflow, expected, "overflow: {value}");
        }
    }

    #[test]
    fn overflow_is_not_inherited() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { overflow: hidden; }"),
        );
        assert_eq!(styles[&div].overflow, super::Overflow::Hidden);
        assert_eq!(styles[&p].overflow, super::Overflow::Visible);
    }

    #[test]
    fn box_sizing_parses_both_keywords_and_defaults_to_content_box() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&div].box_sizing, super::BoxSizing::ContentBox);

        for (value, expected) in [
            ("content-box", super::BoxSizing::ContentBox),
            ("border-box", super::BoxSizing::BorderBox),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ box-sizing: {value}; }}")),
            );
            assert_eq!(styles[&div].box_sizing, expected, "box-sizing: {value}");
        }
    }

    #[test]
    fn box_sizing_is_not_inherited() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { box-sizing: border-box; }"),
        );
        assert_eq!(styles[&div].box_sizing, super::BoxSizing::BorderBox);
        assert_eq!(styles[&p].box_sizing, super::BoxSizing::ContentBox);
    }

    #[test]
    fn z_index_parses_auto_and_integers_and_is_not_inherited() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&div].z_index, super::ZIndex::Auto);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { z-index: -3; }"),
        );
        assert_eq!(styles[&div].z_index, super::ZIndex::Value(-3));
        assert_eq!(
            styles[&p].z_index,
            super::ZIndex::Auto,
            "z-index should not be inherited"
        );
    }

    #[test]
    fn visibility_parses_all_keywords_and_is_inherited() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        for (value, expected) in [
            ("visible", super::Visibility::Visible),
            ("hidden", super::Visibility::Hidden),
            ("collapse", super::Visibility::Collapse),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ visibility: {value}; }}")),
            );
            assert_eq!(styles[&div].visibility, expected, "visibility: {value}");
            assert_eq!(
                styles[&p].visibility, expected,
                "visibility should be inherited: {value}"
            );
        }
    }

    #[test]
    fn outline_shorthand_expands_to_width_style_color_and_defaults_to_currentcolor() {
        let dom = html::parse(br#"<div style="color: rgb(9, 9, 9);">a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { outline: 3px dashed; }"),
        );
        assert_eq!(styles[&div].outline_width.0, 3.0);
        assert_eq!(styles[&div].outline_style, super::BorderStyle::Dashed);
        // 色を省略した場合は`currentcolor`(この要素自身のcolor)へ解決される。
        assert_eq!(
            styles[&div].outline_color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            }
        );
    }

    #[test]
    fn border_style_keyword_parses_groove_ridge_inset_outset() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for (value, expected) in [
            ("groove", super::BorderStyle::Groove),
            ("ridge", super::BorderStyle::Ridge),
            ("inset", super::BorderStyle::Inset),
            ("outset", super::BorderStyle::Outset),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ border-style: {value}; }}")),
            );
            assert_eq!(
                styles[&div].border_top_style, expected,
                "border-style: {value}"
            );
        }
    }

    #[test]
    fn border_radius_shorthand_with_slash_sets_independent_horizontal_and_vertical_radii() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { border-radius: 10px 20px / 30px 40px; }"),
        );
        let style = &styles[&div];
        assert_eq!(style.border_top_left_radius.horizontal.0, 10.0);
        assert_eq!(style.border_top_left_radius.vertical.0, 30.0);
        assert_eq!(style.border_top_right_radius.horizontal.0, 20.0);
        assert_eq!(style.border_top_right_radius.vertical.0, 40.0);
        // 2値指定は(top-left/bottom-right, top-right/bottom-left)の順。
        assert_eq!(style.border_bottom_right_radius.horizontal.0, 10.0);
        assert_eq!(style.border_bottom_right_radius.vertical.0, 30.0);
    }

    #[test]
    fn border_radius_shorthand_without_slash_makes_a_circle() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { border-radius: 15px; }"),
        );
        let corner = styles[&div].border_top_left_radius;
        assert_eq!(corner.horizontal.0, 15.0);
        assert_eq!(corner.vertical.0, 15.0);
    }

    #[test]
    fn border_corner_radius_longhand_accepts_one_or_two_lengths() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { border-top-left-radius: 5px 8px; border-top-right-radius: 6px; }",
            ),
        );
        let style = &styles[&div];
        assert_eq!(style.border_top_left_radius.horizontal.0, 5.0);
        assert_eq!(style.border_top_left_radius.vertical.0, 8.0);
        assert_eq!(style.border_top_right_radius.horizontal.0, 6.0);
        assert_eq!(style.border_top_right_radius.vertical.0, 6.0);
    }

    #[test]
    fn content_attr_reads_the_element_own_html_attribute() {
        let dom = html::parse(br#"<span data-label="hello">x</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(r#"span::before { content: attr(data-label) ": "; }"#),
        );
        assert_eq!(
            styles[&span].pseudo_before_content.as_deref(),
            Some("hello: ")
        );
    }

    #[test]
    fn content_attr_is_empty_when_the_attribute_is_missing() {
        let dom = html::parse(br#"<span>x</span>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(r#"span::before { content: "[" attr(data-missing) "]"; }"#),
        );
        assert_eq!(styles[&span].pseudo_before_content.as_deref(), Some("[]"));
    }

    #[test]
    fn counter_increments_across_siblings_and_resets_are_scoped_to_the_parent() {
        // 兄弟間ではカウンタが引き継がれ(counter-incrementが累積する)、
        // 親が異なればcounter-resetにより独立したスコープになる([0024]決定2)。
        let dom = html::parse(
            br#"<div>
                <section>
                    <h2 class="a">a</h2>
                    <h2 class="b">b</h2>
                </section>
                <section>
                    <h2 class="c">c</h2>
                </section>
            </div>"#,
        );
        let mut h2s = Vec::new();
        find_all(&dom, dom.document(), "h2", &mut h2s);
        assert_eq!(h2s.len(), 3);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "section { counter-reset: h2count; } \
                 h2 { counter-increment: h2count; } \
                 h2::before { content: counter(h2count) \". \"; }",
            ),
        );
        assert_eq!(
            styles[&h2s[0]].pseudo_before_content.as_deref(),
            Some("1. ")
        );
        assert_eq!(
            styles[&h2s[1]].pseudo_before_content.as_deref(),
            Some("2. ")
        );
        // 2つ目の`section`は独立したスコープなので1から数え直す。
        assert_eq!(
            styles[&h2s[2]].pseudo_before_content.as_deref(),
            Some("1. ")
        );
    }

    #[test]
    fn counter_reset_on_an_element_stays_visible_to_its_following_siblings() {
        // 回帰テスト: 実装当初、`counter-reset`をpushした要素自身の処理が
        // 終わった時点で即popしてしまい、後続の兄弟要素からカウンタが
        // 見えなくなるバグがあった([0024]決定2、「スコープは要素自身とそれに
        // 続く兄弟要素まで及ぶ」)。
        let dom = html::parse(
            br#"<div>
                <h2 class="reset">Intro</h2>
                <h3 class="a">A</h3>
                <h3 class="b">B</h3>
            </div>"#,
        );
        let h3_a = find(&dom, dom.document(), "h3").expect("h3 not found");
        let mut h3s = Vec::new();
        find_all(&dom, dom.document(), "h3", &mut h3s);
        assert_eq!(h3s[0], h3_a);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "h2 { counter-reset: section; } \
                 h3 { counter-increment: section; } \
                 h3::before { content: counter(section) \". \"; }",
            ),
        );
        assert_eq!(
            styles[&h3s[0]].pseudo_before_content.as_deref(),
            Some("1. ")
        );
        assert_eq!(
            styles[&h3s[1]].pseudo_before_content.as_deref(),
            Some("2. ")
        );
    }

    #[test]
    fn counters_function_joins_nested_scope_values_with_the_separator() {
        let dom = html::parse(
            br#"<ol class="outer">
                <li class="a">a
                    <ol class="inner"><li class="b">b</li></ol>
                </li>
            </ol>"#,
        );
        let li_a = find(&dom, dom.document(), "li").expect("li not found");
        let mut lis = Vec::new();
        find_all(&dom, dom.document(), "li", &mut lis);
        assert_eq!(lis[0], li_a);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "ol { counter-reset: item; } \
                 li { counter-increment: item; } \
                 li::before { content: counters(item, \".\"); }",
            ),
        );
        assert_eq!(styles[&lis[0]].pseudo_before_content.as_deref(), Some("1"));
        assert_eq!(
            styles[&lis[1]].pseudo_before_content.as_deref(),
            Some("1.1")
        );
    }

    #[test]
    fn counter_increment_on_an_unknown_counter_implicitly_creates_it_at_zero() {
        let dom = html::parse(br#"<div><span>x</span></div>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "span { counter-increment: undeclared; } \
                 span::before { content: counter(undeclared); }",
            ),
        );
        assert_eq!(styles[&span].pseudo_before_content.as_deref(), Some("1"));
    }

    #[test]
    fn counter_styles_cover_roman_alpha_and_non_numeric_fallback() {
        let dom = html::parse(br#"<div><span>x</span></div>"#);
        let span = find(&dom, dom.document(), "span").expect("span not found");

        for (style, expected) in [
            ("upper-roman", "IV"),
            ("lower-roman", "iv"),
            ("upper-alpha", "D"),
            ("lower-alpha", "d"),
            ("decimal-leading-zero", "04"),
            ("disc", ""),
            ("none", ""),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!(
                    "span {{ counter-increment: c 4; }} \
                     span::before {{ content: counter(c, {style}); }}"
                )),
            );
            assert_eq!(
                styles[&span].pseudo_before_content.as_deref(),
                Some(expected),
                "counter(c, {style})"
            );
        }
    }

    #[test]
    fn after_content_is_resolved_after_descendants_so_it_reflects_their_counter_changes() {
        // 回帰テスト: `::after`をDOM順で子孫より先に(この要素自身の処理中に)
        // 解決すると、子孫による`counter-increment`/`quotes`の変更を反映
        // できないバグがあった([0024]決定4関連の実装ノート)。
        let dom = html::parse(br#"<div><span>x</span></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let span = find(&dom, dom.document(), "span").expect("span not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { counter-reset: c; } \
                 span { counter-increment: c; } \
                 div::after { content: \"total=\" counter(c); }",
            ),
        );
        let _ = span;
        assert_eq!(
            styles[&div].pseudo_after_content.as_deref(),
            Some("total=1")
        );
    }

    #[test]
    fn nested_quotes_use_the_pair_matching_their_nesting_depth() {
        // 回帰テスト: `::after`(close-quote)の深度更新が子孫の処理より先に
        // 行われてしまい、ネストした`<q>`が常に深度0のペアを使ってしまう
        // バグがあった。
        let dom = html::parse(br#"<div><q class="outer">a<q class="inner">b</q>c</q></div>"#);
        let outer = find(&dom, dom.document(), "q").expect("outer q not found");
        let mut qs = Vec::new();
        find_all(&dom, dom.document(), "q", &mut qs);
        assert_eq!(qs[0], outer);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                r#"q { quotes: "\201C" "\201D" "\2018" "\2019"; }
                   q::before { content: open-quote; }
                   q::after { content: close-quote; }"#,
            ),
        );
        assert_eq!(
            styles[&qs[0]].pseudo_before_content.as_deref(),
            Some("\u{201C}")
        );
        assert_eq!(
            styles[&qs[1]].pseudo_before_content.as_deref(),
            Some("\u{2018}")
        );
        assert_eq!(
            styles[&qs[1]].pseudo_after_content.as_deref(),
            Some("\u{2019}")
        );
        assert_eq!(
            styles[&qs[0]].pseudo_after_content.as_deref(),
            Some("\u{201D}")
        );
    }

    #[test]
    fn quotes_none_produces_empty_strings_but_still_tracks_depth() {
        let dom = html::parse(br#"<q>a</q>"#);
        let q = find(&dom, dom.document(), "q").expect("q not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "q { quotes: none; } q::before { content: open-quote; } \
                 q::after { content: close-quote; }",
            ),
        );
        assert_eq!(styles[&q].pseudo_before_content.as_deref(), Some(""));
        assert_eq!(styles[&q].pseudo_after_content.as_deref(), Some(""));
    }

    #[test]
    fn first_letter_style_only_captures_the_supported_property_subset() {
        let dom = html::parse(br#"<p>text</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "p::first-letter { font-size: 2em; color: rgb(200, 0, 0); \
                 float: left; }",
            ),
        );
        let fl = styles[&p]
            .first_letter_style
            .as_ref()
            .expect("first_letter_style should be Some");
        assert_eq!(fl.font_size, Some(super::Length(32.0)));
        assert_eq!(
            fl.color,
            Some(RgbaColor {
                red: 200,
                green: 0,
                blue: 0,
                alpha: 1.0
            })
        );
        // `float`はサポート対象外のプロパティなので無視される(既知の簡略化)。
        assert_eq!(fl.font_weight, None);
    }

    #[test]
    fn first_letter_style_is_none_without_a_matching_rule() {
        let dom = html::parse(br#"<p>text</p>"#);
        let p = find(&dom, dom.document(), "p").expect("p not found");
        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(styles[&p].first_letter_style, None);
    }

    #[test]
    fn resolve_margin_box_content_formats_page_and_pages_counters() {
        let parts = vec![
            ContentPart::String("Page ".to_string()),
            ContentPart::Counter("page".to_string(), ListStyleType::Decimal),
            ContentPart::String(" of ".to_string()),
            ContentPart::Counter("pages".to_string(), ListStyleType::Decimal),
        ];
        assert_eq!(
            resolve_margin_box_content(&parts, 3, Some(10)),
            "Page 3 of 10"
        );
    }

    #[test]
    fn resolve_margin_box_content_leaves_pages_empty_when_total_is_unknown() {
        // ストリーミングモードでは`counter(pages)`自体がエラーになる想定
        // ([0028]決定6)だが、この関数自身は`total_pages: None`を渡された
        // 場合に単に空文字列を返すだけの安全な挙動にしておく。
        let parts = vec![ContentPart::Counter(
            "pages".to_string(),
            ListStyleType::Decimal,
        )];
        assert_eq!(resolve_margin_box_content(&parts, 1, None), "");
    }

    #[test]
    fn resolve_margin_box_content_respects_the_counter_style() {
        let parts = vec![ContentPart::Counter(
            "page".to_string(),
            ListStyleType::UpperRoman,
        )];
        assert_eq!(resolve_margin_box_content(&parts, 4, None), "IV");
    }

    #[test]
    fn resolve_margin_box_content_ignores_attr_and_unrelated_counters_and_quotes() {
        let parts = vec![
            ContentPart::Attr("href".to_string()),
            ContentPart::Counter("chapter".to_string(), ListStyleType::Decimal),
            ContentPart::OpenQuote,
            ContentPart::String("x".to_string()),
            ContentPart::CloseQuote,
        ];
        assert_eq!(resolve_margin_box_content(&parts, 1, None), "x");
    }

    /// [0051](../../../docs/decisions/0051-min-max-size-design.md)決定1。
    #[test]
    fn min_and_max_size_parse_lengths_percentages_and_none() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&div].min_width, LengthPercentage::Length(0.0));
        assert_eq!(defaults[&div].min_height, LengthPercentage::Length(0.0));
        assert_eq!(defaults[&div].max_width, MaxSize::None);
        assert_eq!(defaults[&div].max_height, MaxSize::None);

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { min-width: 10px; min-height: 50%; max-width: 20em; max-height: none; }",
            ),
        );
        assert_eq!(styles[&div].min_width, LengthPercentage::Length(10.0));
        assert_eq!(styles[&div].min_height, LengthPercentage::Percentage(0.5));
        // `em`はカスケード時に既定font-size(16px)基準でpxへ畳まれる。
        assert_eq!(
            styles[&div].max_width,
            MaxSize::LengthPercentage(LengthPercentage::Length(320.0))
        );
        assert_eq!(styles[&div].max_height, MaxSize::None);
    }

    /// キーワード値(`auto`/`min-content`等)は非対応で、宣言ごと無視される
    /// ([0051]決定1)。同じルール内の他の宣言には影響しない。
    #[test]
    fn min_and_max_size_reject_intrinsic_sizing_keywords() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { min-width: auto; max-width: max-content; min-height: min-content; \
                 max-height: fit-content; width: 30px; }",
            ),
        );
        assert_eq!(styles[&div].min_width, LengthPercentage::Length(0.0));
        assert_eq!(styles[&div].min_height, LengthPercentage::Length(0.0));
        assert_eq!(styles[&div].max_width, MaxSize::None);
        assert_eq!(styles[&div].max_height, MaxSize::None);
        assert_eq!(
            styles[&div].width,
            LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(30.0)),
            "unsupported keywords must not swallow the other declarations"
        );
    }

    /// [0052](../../../docs/decisions/0052-aspect-ratio-design.md)決定1。
    #[test]
    fn aspect_ratio_parses_auto_ratios_and_their_combination() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert_eq!(defaults[&div].aspect_ratio, AspectRatio::default());
        assert!(defaults[&div].aspect_ratio.auto);
        assert_eq!(defaults[&div].aspect_ratio.ratio, None);

        for (value, expected) in [
            (
                "auto",
                AspectRatio {
                    auto: true,
                    ratio: None,
                },
            ),
            (
                "16 / 9",
                AspectRatio {
                    auto: false,
                    ratio: Some(16.0 / 9.0),
                },
            ),
            // 分母省略は`/ 1`。
            (
                "2",
                AspectRatio {
                    auto: false,
                    ratio: Some(2.0),
                },
            ),
            (
                "auto 16 / 9",
                AspectRatio {
                    auto: true,
                    ratio: Some(16.0 / 9.0),
                },
            ),
            // `auto`と`<ratio>`は順序を問わない。
            (
                "16 / 9 auto",
                AspectRatio {
                    auto: true,
                    ratio: Some(16.0 / 9.0),
                },
            ),
        ] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ aspect-ratio: {value}; }}")),
            );
            assert_eq!(styles[&div].aspect_ratio, expected, "aspect-ratio: {value}");
        }
    }

    /// 0や負の数を含む比(degenerate ratio)は無効な宣言として無視する。
    #[test]
    fn aspect_ratio_rejects_degenerate_ratios() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        for value in ["0 / 1", "1 / 0", "-16 / 9", "0"] {
            let styles = compute_styles(
                &dom,
                &Stylesheet::default(),
                &parse_stylesheet(&format!("div {{ aspect-ratio: {value}; width: 30px; }}")),
            );
            assert_eq!(
                styles[&div].aspect_ratio,
                AspectRatio::default(),
                "aspect-ratio: {value} should be ignored"
            );
            assert_eq!(
                styles[&div].width,
                LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(30.0)),
                "an invalid ratio must not swallow the other declarations"
            );
        }
    }

    /// [0053](../../../docs/decisions/0053-text-details-design.md)決定1。
    #[test]
    fn text_detail_properties_parse_and_inherit() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let defaults = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());
        assert!(defaults[&div].text_shadow.is_empty());
        assert_eq!(defaults[&div].text_overflow, TextOverflow::Clip);
        assert_eq!(defaults[&div].word_break, WordBreak::Normal);
        assert_eq!(defaults[&div].overflow_wrap, OverflowWrap::Normal);
        assert_eq!(defaults[&div].hyphens, Hyphens::Manual);
        assert_eq!(defaults[&div].text_emphasis_style, EmphasisStyle::None);
        assert_eq!(
            defaults[&div].text_emphasis_position,
            EmphasisPosition::Over
        );

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(
                "div { text-shadow: 1px 2px 3px rgb(1, 2, 3); word-break: break-all; \
                 overflow-wrap: break-word; hyphens: none; text-overflow: ellipsis; \
                 text-emphasis: open sesame rgb(4, 5, 6); text-emphasis-position: under; }",
            ),
        );
        assert_eq!(styles[&div].text_shadow.len(), 1);
        assert_eq!(styles[&div].text_shadow[0].offset_x, 1.0);
        assert_eq!(styles[&div].text_shadow[0].offset_y, 2.0);
        assert_eq!(styles[&div].text_shadow[0].blur_radius, 3.0);
        assert_eq!(styles[&div].text_shadow[0].color.red, 1);
        assert_eq!(styles[&div].word_break, WordBreak::BreakAll);
        assert_eq!(styles[&div].overflow_wrap, OverflowWrap::BreakWord);
        assert_eq!(styles[&div].hyphens, Hyphens::None);
        assert_eq!(styles[&div].text_overflow, TextOverflow::Ellipsis);
        assert_eq!(
            styles[&div].text_emphasis_style,
            EmphasisStyle::Shape {
                shape: crate::style::EmphasisShape::Sesame,
                filled: false,
            }
        );
        assert_eq!(styles[&div].text_emphasis_color.red, 4);
        assert_eq!(styles[&div].text_emphasis_position, EmphasisPosition::Under);

        // 継承する/しないの区別(`text-overflow`だけ非継承)。
        assert_eq!(styles[&p].text_shadow.len(), 1);
        assert_eq!(styles[&p].word_break, WordBreak::BreakAll);
        assert_eq!(styles[&p].overflow_wrap, OverflowWrap::BreakWord);
        assert_eq!(styles[&p].hyphens, Hyphens::None);
        assert_eq!(styles[&p].text_emphasis_position, EmphasisPosition::Under);
        assert_eq!(styles[&p].text_emphasis_color.red, 4);
        assert_eq!(
            styles[&p].text_overflow,
            TextOverflow::Clip,
            "text-overflow must not be inherited"
        );
    }

    /// `word-wrap`は`overflow-wrap`のレガシー別名、`hyphens: auto`は
    /// `manual`と同じ挙動([0053]決定1・決定2)。
    #[test]
    fn text_detail_property_aliases() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { word-wrap: anywhere; hyphens: auto; }"),
        );
        assert_eq!(styles[&div].overflow_wrap, OverflowWrap::BreakWord);
        assert_eq!(styles[&div].hyphens, Hyphens::Manual);
    }

    /// `text-emphasis-style: <string>`は先頭1文字だけを使う(決定1)。
    #[test]
    fn text_emphasis_style_accepts_a_string() {
        let dom = html::parse(br#"<div>a</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet(r#"div { text-emphasis-style: "×か"; }"#),
        );
        assert_eq!(styles[&div].text_emphasis_style, EmphasisStyle::String('×'));
    }

    #[test]
    fn min_and_max_size_are_not_inherited() {
        let dom = html::parse(br#"<div><p>a</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let styles = compute_styles(
            &dom,
            &Stylesheet::default(),
            &parse_stylesheet("div { min-width: 100px; max-height: 40px; }"),
        );
        assert_eq!(styles[&div].min_width, LengthPercentage::Length(100.0));
        assert_eq!(styles[&p].min_width, LengthPercentage::Length(0.0));
        assert_eq!(styles[&p].max_height, MaxSize::None);
    }
}
