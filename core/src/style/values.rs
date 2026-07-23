//! CSSプロパティ値の型。M1で対応する最小セットのみ。
//!
//! `Length`/`LengthPercentage`/`LengthPercentageOrAuto`は、カスケード解決後の
//! **計算値**(常にpx単位に解決済み)を表す。パース直後の**指定値**は
//! `em`/`rem`のような相対単位を区別する必要があるため、代わりに
//! `SpecifiedLength`/`SpecifiedLengthPercentage`/`SpecifiedLengthPercentageOrAuto`
//! を使う(`style::computed`が、要素自身とルート要素の計算済み`font-size`を
//! 使ってこれらをpxへ解決する)。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    /// `table`要素専用。テーブルフォーマッティングコンテキストを確立する
    /// (`table-row`/`table-cell`の子孫を集めて列幅アルゴリズムでレイアウトする)。
    Table,
    /// `tr`要素専用。`Display::Table`の祖先の下でのみ意味を持つ。
    TableRow,
    /// `td`/`th`要素専用。`Display::TableRow`の祖先の下でのみ意味を持つ。
    TableCell,
    /// `caption`要素専用。`Display::Table`の祖先の下でのみ意味を持つ
    /// (`box_tree.rs::collect_table_rows`が`table-row`と並んで検出する)。
    TableCaption,
    /// `li`要素の既定値([0022](../../../docs/decisions/0022-list-style-design.md))。
    /// 通常のブロックボックスに加えてマーカーボックス(箇条書きの記号・番号)を
    /// 生成する。`box_tree.rs::child_kind`では`Block`と同様に扱う。
    ListItem,
    /// `flex`要素専用。Flexboxフォーマッティングコンテキストを確立する
    /// (子要素ごとに1個のflexアイテムを生成し、taffyへレイアウトを委譲する、
    /// [0034](../../../docs/decisions/0034-flexbox-design.md)決定1)。
    /// `inline-flex`は非対応(決定4、既知の簡略化)。
    Flex,
    None,
}

/// `font-weight`。数値指定(`700`等)は600以上を`Bold`として扱う簡略実装。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/// `font-style`。`oblique`は専用の傾斜を持たないため`Italic`と同一視する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// `text-decoration-line`。`underline`と`line-through`は同時指定可能
/// (仕様通り)。`overline`(あまり使われないため)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub line_through: bool,
}

/// `border-style`。`groove`/`ridge`/`inset`/`outset`(border-colorから2階調の
/// 疑似立体陰影を算出する)は[0023](../../../docs/decisions/0023-box-model-details-design.md)
/// 決定5で対応(既存の非対応方針から変更、ユーザー確認済み)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

/// `break-before`/`break-after`の値。CSS仕様上`page`は複数ページサイズ/
/// 名前付きページ対応を見据えた別キーワードだが、単一ページサイズしか
/// 扱わない現状のスコープでは`always`と同じ「強制的に新しいページへ送る」
/// 効果として扱う。`left`/`right`/`recto`/`verso`(見開き制御)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakBetween {
    #[default]
    Auto,
    Avoid,
    Always,
}

/// `break-inside`の値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
}

/// `float`(CSS2.1 9.5.1)。`inline-start`/`inline-end`論理値はCSS2.1の
/// スコープ外のため非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

/// `clear`(CSS2.1 9.5.2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

/// `position`。`absolute`/`fixed`は
/// [0018](../../../docs/decisions/0018-css21-css3-coverage-strategy.md)で
/// 帳票用途での必要性が`relative`より低いと判断し非対応(既存の`border-style`
/// groove/ridge等と同じパターンで、パース時に宣言ごと無視する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
}

/// `text-align`。`start`/`end`(bidi対応)は非対応、`direction`自体が非対応のため
/// スコープ外。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Right,
    Center,
    Justify,
}

/// `white-space`。`pre-wrap`/`pre-line`/`break-spaces`は非対応
/// (帳票用途で必要になるのは`pre`までと判断、[0020](
/// ../../../docs/decisions/0020-typography-details-design.md)決定4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
}

/// `text-transform`。`full-width`/`full-size-kana`(日本語組版の特殊変換)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// `line-height`のパース直後の指定値。`<number>`/`<percentage>`はCSS仕様上
/// 「computed valueは指定値の数値そのもの」(親のfont-sizeで先に乗算した絶対値
/// ではない)という他の継承プロパティとは異なる規則を持つ
/// ([0020](../../../docs/decisions/0020-typography-details-design.md)決定3)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedLineHeight {
    Normal,
    /// `<number>`。
    Number(f32),
    Length(SpecifiedLength),
    /// `<percentage>`。`<number>`と同じ意味(50%は0.5と同義)。
    Percentage(f32),
}

/// `letter-spacing`/`word-spacing`共通の指定値(どちらも`normal | <length>`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedSpacing {
    Normal,
    Length(SpecifiedLength),
}

impl SpecifiedSpacing {
    /// `normal`は`0`として解決する(単語間・文字間の追加スペースなし)。
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Length(length) => length.resolve(font_size, root_font_size).0,
        }
    }
}

/// `border-collapse`。collapse値は見た目の枠線描画のみ統合し、レイアウト計算
/// (列幅・セル配置)はseparateと完全に同一に保つ([0021](
/// ../../../docs/decisions/0021-table-layout-design.md)決定1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

/// `caption-side`。CSS2.1の`left`/`right`(縦書き対応の論理値)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptionSide {
    #[default]
    Top,
    Bottom,
}

/// `table-layout`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    Fixed,
}

/// `empty-cells`。`border-collapse: separate`でのみ意味を持つ(CSS仕様通り)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmptyCells {
    #[default]
    Show,
    Hide,
}

/// `vertical-align`(テーブルセル文脈専用と割り切る、インライン文脈の
/// `vertical-align`は非対応)。CSS2.1の初期値は`baseline`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
    #[default]
    Baseline,
}

/// `list-style-type`([0022](../../../docs/decisions/0022-list-style-design.md)決定2)。
/// `disc`/`circle`/`square`は固定記号、それ以外はカウンタ値から生成する
/// (数値・アルファベット系は「本体+`.`」形式、既知の簡略化)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
    None,
}

/// `content`の1パーツ(パース時点、まだ解決されていない値)。[0024](
/// ../../../docs/decisions/0024-generated-content-design.md)決定1。複数パーツを
/// 連結できる(`content: "Chapter " counter(chapter) ": "`等)。
#[derive(Debug, Clone, PartialEq)]
pub enum ContentPart {
    String(String),
    /// `attr(name)`。HTML属性値。
    Attr(String),
    /// `counter(name [, style])`。`style`は[`ListStyleType`]を再利用する
    /// (`disc`/`circle`/`square`/`none`は空文字列を生成する、仕様通り)。
    Counter(String, ListStyleType),
    /// `counters(name, separator [, style])`。
    Counters(String, String, ListStyleType),
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
}

/// `quotes`の1階層分(開き引用符, 閉じ引用符)。[0024]決定3。
#[derive(Debug, Clone, PartialEq)]
pub struct QuotePair {
    pub open: String,
    pub close: String,
}

/// `list-style-position`([0022](../../../docs/decisions/0022-list-style-design.md)決定4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

/// `overflow`([0023](../../../docs/decisions/0023-box-model-details-design.md)決定1)。
/// `scroll`/`auto`は`hidden`と区別せず同じクリップ処理として扱う(印刷に
/// スクロールの概念が無いため)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl Overflow {
    /// `visible`以外は全て同じクリップ処理の対象になる(決定1)。
    pub fn clips(self) -> bool {
        self != Overflow::Visible
    }
}

/// `visibility`([0023](../../../docs/decisions/0023-box-model-details-design.md)決定4)。
/// `collapse`は`hidden`と同一視する(テーブル行/列の高さ再計算は非対応)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    pub fn is_hidden(self) -> bool {
        self != Visibility::Visible
    }
}

/// `box-sizing`([0027](../../../docs/decisions/0027-box-sizing-design.md))。
/// `padding-box`(標準外)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// `z-index`([0023](../../../docs/decisions/0023-box-model-details-design.md)決定2)。
/// `position: static`の要素には効果を持たない(仕様通り、呼び出し側が判定する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZIndex {
    #[default]
    Auto,
    Value(i32),
}

impl ZIndex {
    /// 描画順のソートキーとして使う実効値(`auto`は`0`と同義、仕様通り)。
    pub fn sort_key(self) -> i32 {
        match self {
            ZIndex::Auto => 0,
            ZIndex::Value(v) => v,
        }
    }
}

/// 長さ(px)またはパーセンテージ。カスケード解決済みの計算値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentage {
    Length(f32),
    Percentage(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentageOrAuto {
    LengthPercentage(LengthPercentage),
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Length(pub f32);

/// パース直後の長さの指定値。`em`は基準フォントサイズ(呼び出し側が要素自身の
/// ものか親のものかを選ぶ)に対する相対値、`rem`はルート要素のフォントサイズに
/// 対する相対値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedLength {
    Px(f32),
    Em(f32),
    Rem(f32),
}

impl SpecifiedLength {
    /// `font_size`(em基準、px)と`root_font_size`(rem基準、px)を使って
    /// 計算値の[`Length`]へ解決する。
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> Length {
        match self {
            Self::Px(px) => Length(px),
            Self::Em(em) => Length(em * font_size),
            Self::Rem(rem) => Length(rem * root_font_size),
        }
    }
}

/// `border-radius`の1コーナー分の計算値(水平半径, 垂直半径)。真円は
/// 水平=垂直([0023](../../../docs/decisions/0023-box-model-details-design.md)決定6)。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadius {
    pub horizontal: Length,
    pub vertical: Length,
}

/// パース直後の`border-radius`1コーナー分の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecifiedCornerRadius {
    pub horizontal: SpecifiedLength,
    pub vertical: SpecifiedLength,
}

impl SpecifiedCornerRadius {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> CornerRadius {
        CornerRadius {
            horizontal: self.horizontal.resolve(font_size, root_font_size),
            vertical: self.vertical.resolve(font_size, root_font_size),
        }
    }
}

/// パース直後の「長さまたはパーセンテージ」の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedLengthPercentage {
    Length(SpecifiedLength),
    Percentage(f32),
}

impl SpecifiedLengthPercentage {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> LengthPercentage {
        match self {
            Self::Length(length) => {
                LengthPercentage::Length(length.resolve(font_size, root_font_size).0)
            }
            Self::Percentage(fraction) => LengthPercentage::Percentage(fraction),
        }
    }
}

/// パース直後の「長さ・パーセンテージまたはauto」の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedLengthPercentageOrAuto {
    LengthPercentage(SpecifiedLengthPercentage),
    Auto,
}

impl SpecifiedLengthPercentageOrAuto {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> LengthPercentageOrAuto {
        match self {
            Self::Auto => LengthPercentageOrAuto::Auto,
            Self::LengthPercentage(lp) => {
                LengthPercentageOrAuto::LengthPercentage(lp.resolve(font_size, root_font_size))
            }
        }
    }
}

/// `background-position`の計算値(水平/垂直、border-box基準の長さまたは
/// パーセンテージ)。キーワード(`left`/`center`/`right`/`top`/`bottom`)は
/// パース時点で対応するパーセンテージへ解決済み([0025](
/// ../../../docs/decisions/0025-background-details-design.md)決定1)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    pub horizontal: LengthPercentage,
    pub vertical: LengthPercentage,
}

impl Default for BackgroundPosition {
    /// 初期値`0% 0%`(左上)。
    fn default() -> Self {
        Self {
            horizontal: LengthPercentage::Percentage(0.0),
            vertical: LengthPercentage::Percentage(0.0),
        }
    }
}

/// パース直後の`background-position`の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecifiedBackgroundPosition {
    pub horizontal: SpecifiedLengthPercentage,
    pub vertical: SpecifiedLengthPercentage,
}

impl SpecifiedBackgroundPosition {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> BackgroundPosition {
        BackgroundPosition {
            horizontal: self.horizontal.resolve(font_size, root_font_size),
            vertical: self.vertical.resolve(font_size, root_font_size),
        }
    }
}

/// `background-size`の計算値。`Cover`/`Contain`はintrinsicサイズに基づき
/// 描画時([0025]決定3)に矩形へ変換する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    WidthHeight(LengthPercentageOrAuto, LengthPercentageOrAuto),
    Cover,
    Contain,
}

impl Default for BackgroundSize {
    /// 初期値`auto auto`。
    fn default() -> Self {
        Self::WidthHeight(LengthPercentageOrAuto::Auto, LengthPercentageOrAuto::Auto)
    }
}

/// パース直後の`background-size`の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedBackgroundSize {
    WidthHeight(
        SpecifiedLengthPercentageOrAuto,
        SpecifiedLengthPercentageOrAuto,
    ),
    Cover,
    Contain,
}

impl SpecifiedBackgroundSize {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> BackgroundSize {
        match self {
            Self::WidthHeight(w, h) => BackgroundSize::WidthHeight(
                w.resolve(font_size, root_font_size),
                h.resolve(font_size, root_font_size),
            ),
            Self::Cover => BackgroundSize::Cover,
            Self::Contain => BackgroundSize::Contain,
        }
    }
}

/// `background-repeat`。CSS2.1の値集合(repeat/repeat-x/repeat-y/no-repeat)
/// のみ対応(複数背景のカンマ区切り記法・`round`/`space`はCSS3スコープ外)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

/// `background-attachment`。`fixed`は`scroll`と同一視して描画する
/// ([0025]決定5、既知の簡略化)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundAttachment {
    #[default]
    Scroll,
    Fixed,
}

/// 色。`currentcolor`の解決や継承は計算スタイル(T4)の役割なので、
/// ここではパース結果をそのまま保持する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    CurrentColor,
    Rgba {
        red: u8,
        green: u8,
        blue: u8,
        alpha: f32,
    },
}

/// `object-fit`。`<img>`(置換要素)専用、非継承プロパティ。
/// [0030](../../../docs/decisions/0030-object-fit-position-design.md)参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    /// 初期値。intrinsicアスペクト比を無視してcontent-box全体に引き伸ばす
    /// (`object-fit`未対応時の既存の`<img>`描画と同じ挙動)。
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// `box-shadow`の1つ分。パース直後の指定値(長さは`em`/`rem`未解決)。
/// カンマ区切りの複数指定は`Vec<SpecifiedBoxShadow>`で保持する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecifiedBoxShadow {
    pub offset_x: SpecifiedLength,
    pub offset_y: SpecifiedLength,
    pub blur_radius: SpecifiedLength,
    pub spread_radius: SpecifiedLength,
    /// 省略時は`currentcolor`相当(`None`のまま計算スタイル側で解決する)。
    pub color: Option<Color>,
    /// `inset`キーワード。パースはするが描画は非対応
    /// ([0032](../../../docs/decisions/0032-box-shadow-design.md)決定、
    /// 既知の簡略化)。
    pub inset: bool,
}

impl SpecifiedBoxShadow {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> BoxShadow {
        BoxShadow {
            offset_x: self.offset_x.resolve(font_size, root_font_size).0,
            offset_y: self.offset_y.resolve(font_size, root_font_size).0,
            blur_radius: self.blur_radius.resolve(font_size, root_font_size).0,
            spread_radius: self.spread_radius.resolve(font_size, root_font_size).0,
            color: self.color,
            inset: self.inset,
        }
    }
}

/// `box-shadow`の1つ分。長さはpx解決済みだが、`color`は`currentcolor`が
/// 未解決のまま(`RgbaColor`への解決は計算スタイルの役割)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Option<Color>,
    pub inset: bool,
}

/// `flex-direction`([0034](../../../docs/decisions/0034-flexbox-design.md)決定4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// `flex-wrap`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// `justify-content`。CSS Box Alignment仕様の`safe`/`unsafe`オーバーフロー
/// キーワードは非対応(既知の簡略化、実務上ほぼ使われない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// `align-items`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    #[default]
    Stretch,
}

/// `align-content`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    #[default]
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// `align-self`。`Auto`(初期値)は親の`align-items`をそのまま使う(仕様通り)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

/// `flex-basis`のパース直後の指定値(`em`/`rem`未解決)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedFlexBasis {
    Auto,
    /// `content`キーワード。本実装では`Auto`と同一視する(既知の簡略化、
    /// [0034](../../../docs/decisions/0034-flexbox-design.md))。
    Content,
    LengthPercentage(SpecifiedLengthPercentage),
}

impl SpecifiedFlexBasis {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> FlexBasis {
        match self {
            Self::Auto | Self::Content => FlexBasis::Auto,
            Self::LengthPercentage(lp) => {
                FlexBasis::LengthPercentage(lp.resolve(font_size, root_font_size))
            }
        }
    }
}

/// `flex-basis`の計算値。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FlexBasis {
    #[default]
    Auto,
    LengthPercentage(LengthPercentage),
}
