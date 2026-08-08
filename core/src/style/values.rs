//! CSSプロパティ値の型。
//!
//! `Length`/`LengthPercentage`/`LengthPercentageOrAuto`は、カスケード解決後の
//! 計算値(常にpx単位に解決済み)を表す。パース直後の指定値は
//! `em`/`rem`のような相対単位を区別する必要があるため、代わりに
//! `SpecifiedLength`/`SpecifiedLengthPercentage`/`SpecifiedLengthPercentageOrAuto`
//! を使う(`style::computed`が、要素自身とルート要素の計算済み`font-size`を
//! 使ってこれらをpxへ解決する)。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    /// 外側はインラインレベル(親の行に参加する)だが、中身はブロックとして
    /// レイアウトされる分割不可能な箱。
    InlineBlock,
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
    /// `li`要素の既定値。通常のブロックボックスに加えてマーカーボックス
    /// (箇条書きの記号・番号)を生成する。
    /// `box_tree.rs::child_kind`では`Block`と同様に扱う。
    ListItem,
    /// `flex`要素専用。Flexboxフォーマッティングコンテキストを確立する
    /// (子要素ごとに1個のflexアイテムを生成し、taffyへレイアウトを委譲する)。
    /// `inline-flex`は非対応。
    Flex,
    /// `display: grid`。`inline-grid`は非対応(`inline-flex`と同じ理由)。
    Grid,
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
/// 疑似立体陰影を算出する)にも対応する。
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

/// `position`。`absolute`/`fixed`にも対応する(`Mode::Streaming`では
/// 無視)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    /// 最も近いpositioned祖先(無ければinitial containing block)を基準に配置。
    Absolute,
    /// 各ページのコンテンツ領域を基準に、全ページへ繰り返し配置。
    Fixed,
}

impl Position {
    /// フロー外に配置される(通常フローのスペースを占めない)positioning か。
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Position::Absolute | Position::Fixed)
    }
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

/// `white-space`。`pre-wrap`/`pre-line`/`break-spaces`は非対応(帳票用途で
/// 必要になるのは`pre`までと判断)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
}

/// `<track-breadth>`の計算値。長さはpx解決済み。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackBreadth {
    Length(f32),
    /// パーセンテージ(50% = 0.5)。
    Percentage(f32),
    /// `<flex>`(`1fr`)。トラックの伸長係数。
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
}

/// `<track-size>`の計算値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackSize {
    Breadth(TrackBreadth),
    /// `minmax(min, max)`。CSS仕様上、min側に`fr`は書けない(パースで拒否する)。
    MinMax(TrackBreadth, TrackBreadth),
    /// `fit-content(<length-percentage>)`。
    FitContent(LengthPercentage),
}

/// `repeat()`の繰り返し回数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatCount {
    Count(u16),
    AutoFill,
    AutoFit,
}

/// `<track-list>`の1要素(単一トラック、または`repeat()`)。
#[derive(Debug, Clone, PartialEq)]
pub enum TrackComponent {
    Single(TrackSize),
    Repeat {
        count: RepeatCount,
        tracks: Vec<TrackSize>,
        /// 繰り返しトラック間のライン名(`tracks.len() + 1`要素)。
        line_names: Vec<Vec<String>>,
    },
}

/// `grid-template-columns`/`grid-template-rows`の計算値。空なら`none`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackList {
    pub components: Vec<TrackComponent>,
    /// トラックの前後に置かれたライン名(`[name]`)。`components.len() + 1`要素。
    pub line_names: Vec<Vec<String>>,
}

/// `grid-row-start`等の配置指定。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum GridLine {
    #[default]
    Auto,
    /// 1-indexedのライン番号(負値は末尾からの数え)。
    Line(i16),
    /// `span <integer>`。
    Span(u16),
    /// 名前付きライン(`grid-template-areas`が暗黙に作る`foo-start`等も含む)。
    Named(String),
    /// `span <custom-ident>`。
    NamedSpan(String, u16),
}

/// `grid-auto-flow`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// `grid-template-areas`が定義する名前付きエリア1つ分。行・列は1-indexedの
/// グリッドライン番号(taffyの`GridTemplateArea`と同じ規約で、
/// `row_end`/`column_end`は終端セルの次のラインを指す)。
#[derive(Debug, Clone, PartialEq)]
pub struct GridArea {
    pub name: String,
    pub row_start: u16,
    pub row_end: u16,
    pub column_start: u16,
    pub column_end: u16,
}

/// `word-break`。改行機会そのものを切り替える。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreak {
    /// 従来どおり: CJK文字が隣接する境界のみ改行可。
    #[default]
    Normal,
    /// すべての文字境界で改行可。
    BreakAll,
    /// CJK境界でも改行しない(空白のみが改行機会)。
    KeepAll,
}

/// `overflow-wrap`(別名`word-wrap`)。改行機会は増やさず、「行頭に置いてもなお
/// 収まらない」場合のフォールバックとして働く。`anywhere`は`break-word`と
/// 同一視する(min-content幅への影響の違いだけで、本エンジンはその区別を
/// 持たない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
}

/// `hyphens`。`auto`は辞書を持たないため`manual`と同じ挙動(soft
/// hyphen(U+00AD)でのみ分割する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hyphens {
    /// soft hyphenでも分割しない。
    None,
    /// soft hyphenを改行機会として扱い、分割時にハイフンを表示する。
    #[default]
    Manual,
}

/// `text-overflow`。`<string>`指定は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

/// `text-emphasis-style`のマーク形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmphasisShape {
    #[default]
    Dot,
    Circle,
    DoubleCircle,
    Triangle,
    Sesame,
}

/// `text-emphasis-style`。`None`はマークなし(初期値)。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum EmphasisStyle {
    #[default]
    None,
    /// キーワード指定。`filled`(塗り)か`open`(輪郭のみ)かと形状の組。
    Shape { shape: EmphasisShape, filled: bool },
    /// `<string>`指定。先頭1文字だけを使う(仕様通り)。
    String(char),
}

/// `text-emphasis-position`。横書きでは`over`/`under`のみが意味を持つ
/// (`right`/`left`は読み飛ばす)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmphasisPosition {
    #[default]
    Over,
    Under,
}

/// `text-shadow`の1つ分。パース直後の指定値(長さは`em`/`rem`未解決)。
/// `box-shadow`と違いspread・insetを持たない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecifiedTextShadow {
    pub offset_x: SpecifiedLength,
    pub offset_y: SpecifiedLength,
    pub blur_radius: SpecifiedLength,
    /// 省略時は`currentcolor`相当(`None`のまま計算スタイル側で解決する)。
    pub color: Option<Color>,
}

impl SpecifiedTextShadow {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> TextShadow {
        TextShadow {
            offset_x: self.offset_x.resolve(font_size, root_font_size).0,
            offset_y: self.offset_y.resolve(font_size, root_font_size).0,
            blur_radius: self.blur_radius.resolve(font_size, root_font_size).0,
            color: self.color,
        }
    }
}

/// `text-shadow`1つ分の計算値(長さはpx解決済み、色は未解決)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: Option<Color>,
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
/// ではない)という他の継承プロパティとは異なる規則を持つ。
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
/// (列幅・セル配置)はseparateと完全に同一に保つ。
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

/// `vertical-align`。CSS2.1の初期値は`baseline`。
///
/// インライン文脈とテーブルセル文脈で値の集合を共有する。テーブルセルに
/// 適用できるのはCSS2.1どおり`top`/`middle`/`bottom`/`baseline`のみで、それ
/// 以外の値を指定した場合は`baseline`として扱う。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
    #[default]
    Baseline,
    /// 下付き。フォントサイズは変えない(縮小は
    /// UAスタイルシートの`sub`規則が担う)。
    Sub,
    /// 上付き(同上)。
    Super,
    /// 親(行の基準ラン)の文字上端に揃える。
    TextTop,
    /// 同じく文字下端に揃える。
    TextBottom,
    /// 長さ・パーセンテージ(正で上方向)。パーセンテージはそのランの
    /// `line-height`基準。
    LengthPercentage(LengthPercentage),
}

/// パース直後の`vertical-align`の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedVerticalAlign {
    Top,
    Middle,
    Bottom,
    Baseline,
    Sub,
    Super,
    TextTop,
    TextBottom,
    LengthPercentage(SpecifiedLengthPercentage),
}

impl SpecifiedVerticalAlign {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> VerticalAlign {
        match self {
            Self::Top => VerticalAlign::Top,
            Self::Middle => VerticalAlign::Middle,
            Self::Bottom => VerticalAlign::Bottom,
            Self::Baseline => VerticalAlign::Baseline,
            Self::Sub => VerticalAlign::Sub,
            Self::Super => VerticalAlign::Super,
            Self::TextTop => VerticalAlign::TextTop,
            Self::TextBottom => VerticalAlign::TextBottom,
            Self::LengthPercentage(lp) => {
                VerticalAlign::LengthPercentage(lp.resolve(font_size, root_font_size))
            }
        }
    }
}

/// `list-style-type`。`disc`/`circle`/`square`は固定記号、それ以外はカウンタ
/// 値から生成する(数値・アルファベット系は「本体+`.`」形式、既知の簡略化)。
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

/// `content`の1パーツ(パース時点、まだ解決されていない値)。複数パーツを
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

/// `quotes`の1階層分(開き引用符, 閉じ引用符)。
#[derive(Debug, Clone, PartialEq)]
pub struct QuotePair {
    pub open: String,
    pub close: String,
}

/// `list-style-position`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

/// `overflow`。`scroll`/`auto`は`hidden`と区別せず同じクリップ処理として扱う
/// (印刷にスクロールの概念が無いため)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl Overflow {
    /// `visible`以外は全て同じクリップ処理の対象になる。
    pub fn clips(self) -> bool {
        self != Overflow::Visible
    }
}

/// `visibility`。`collapse`は`hidden`と同一視する
/// (テーブル行/列の高さ再計算は非対応)。
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

/// `box-sizing`。`padding-box`(標準外)は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// `z-index`。`position: static`の要素には効果を
/// 持たない(仕様通り、呼び出し側が判定する)。
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
    /// `calc`の解決済み複合値。使用値は
    /// `px + percent * basis`(`percent`は割合、50% = 0.5)。
    Calc {
        px: f32,
        percent: f32,
    },
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
/// 水平=垂直。
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

/// `calc`の指定値。`em`/`rem`はパース時点で
/// 未解決なので4成分で保持し、`resolve`でpxへ畳む。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpecifiedCalc {
    pub px: f32,
    pub em: f32,
    pub rem: f32,
    /// パーセンテージの割合(50% = 0.5)。
    pub percent: f32,
}

impl SpecifiedCalc {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> LengthPercentage {
        LengthPercentage::Calc {
            px: self.px + self.em * font_size + self.rem * root_font_size,
            percent: self.percent,
        }
    }
}

/// パース直後の「長さまたはパーセンテージ」の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedLengthPercentage {
    Length(SpecifiedLength),
    Percentage(f32),
    /// `calc`。
    Calc(SpecifiedCalc),
}

impl SpecifiedLengthPercentage {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> LengthPercentage {
        match self {
            Self::Length(length) => {
                LengthPercentage::Length(length.resolve(font_size, root_font_size).0)
            }
            Self::Percentage(fraction) => LengthPercentage::Percentage(fraction),
            Self::Calc(calc) => calc.resolve(font_size, root_font_size),
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

/// `max-width`/`max-height`の計算値。`none`(上限なし)を表現する必要があるため
/// `LengthPercentage`とは別の型を持つ。`min-width`/`min-height`は初期値が
/// `0`なので`LengthPercentage`をそのまま使う。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum MaxSize {
    #[default]
    None,
    LengthPercentage(LengthPercentage),
}

/// パース直後の`<track-breadth>`。長さは`em`/`rem`未解決。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedTrackBreadth {
    Length(SpecifiedLength),
    Percentage(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
}

impl SpecifiedTrackBreadth {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> TrackBreadth {
        match self {
            Self::Length(length) => {
                TrackBreadth::Length(length.resolve(font_size, root_font_size).0)
            }
            Self::Percentage(v) => TrackBreadth::Percentage(v),
            Self::Fr(v) => TrackBreadth::Fr(v),
            Self::Auto => TrackBreadth::Auto,
            Self::MinContent => TrackBreadth::MinContent,
            Self::MaxContent => TrackBreadth::MaxContent,
        }
    }
}

/// パース直後の`<track-size>`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedTrackSize {
    Breadth(SpecifiedTrackBreadth),
    MinMax(SpecifiedTrackBreadth, SpecifiedTrackBreadth),
    FitContent(SpecifiedLengthPercentage),
}

impl SpecifiedTrackSize {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> TrackSize {
        match self {
            Self::Breadth(b) => TrackSize::Breadth(b.resolve(font_size, root_font_size)),
            Self::MinMax(min, max) => TrackSize::MinMax(
                min.resolve(font_size, root_font_size),
                max.resolve(font_size, root_font_size),
            ),
            Self::FitContent(lp) => TrackSize::FitContent(lp.resolve(font_size, root_font_size)),
        }
    }
}

/// パース直後の`<track-list>`の1要素。
#[derive(Debug, Clone, PartialEq)]
pub enum SpecifiedTrackComponent {
    Single(SpecifiedTrackSize),
    Repeat {
        count: RepeatCount,
        tracks: Vec<SpecifiedTrackSize>,
        line_names: Vec<Vec<String>>,
    },
}

/// パース直後の`grid-template-columns`/`-rows`。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpecifiedTrackList {
    pub components: Vec<SpecifiedTrackComponent>,
    pub line_names: Vec<Vec<String>>,
}

impl SpecifiedTrackList {
    pub fn resolve(&self, font_size: f32, root_font_size: f32) -> TrackList {
        TrackList {
            components: self
                .components
                .iter()
                .map(|component| match component {
                    SpecifiedTrackComponent::Single(size) => {
                        TrackComponent::Single(size.resolve(font_size, root_font_size))
                    }
                    SpecifiedTrackComponent::Repeat {
                        count,
                        tracks,
                        line_names,
                    } => TrackComponent::Repeat {
                        count: *count,
                        tracks: tracks
                            .iter()
                            .map(|size| size.resolve(font_size, root_font_size))
                            .collect(),
                        line_names: line_names.clone(),
                    },
                })
                .collect(),
            line_names: self.line_names.clone(),
        }
    }
}

/// `aspect-ratio: auto || <ratio>`。長さを
/// 含まないため指定値と計算値を分ける必要がない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AspectRatio {
    /// `auto`キーワードの有無。置換要素(`<img>`)では内在比を優先する。
    pub auto: bool,
    /// 指定された比(`width / height`)。`None`は比の指定なし。
    pub ratio: Option<f32>,
}

impl Default for AspectRatio {
    /// 初期値`auto`。
    fn default() -> Self {
        Self {
            auto: true,
            ratio: None,
        }
    }
}

/// パース直後の`max-width`/`max-height`の指定値。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedMaxSize {
    None,
    LengthPercentage(SpecifiedLengthPercentage),
}

impl SpecifiedMaxSize {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> MaxSize {
        match self {
            Self::None => MaxSize::None,
            Self::LengthPercentage(lp) => {
                MaxSize::LengthPercentage(lp.resolve(font_size, root_font_size))
            }
        }
    }
}

/// `background-position`の計算値(水平/垂直、border-box基準の長さまたは
/// パーセンテージ)。キーワード(`left`/`center`/`right`/`top`/`bottom`)は
/// パース時点で対応するパーセンテージへ解決済み。
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
/// 描画時に矩形へ変換する。
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

/// `background-attachment`。`fixed`は`scroll`と同一視して描画する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundAttachment {
    #[default]
    Scroll,
    Fixed,
}

/// 色。`currentcolor`の解決や継承は計算スタイルの役割なので、
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
    /// `inset`キーワード。パースはするが描画は非対応。
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

/// `flex-direction`。
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

/// `justify-content`。CSS Box Alignment仕様の`safe`/`unsafe`オーバーフローは非対応。
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
    /// `content`キーワード。`Auto`と同一視する。
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

/// `transform`関数1つ分のパース直後の指定値(`em`/`rem`未解決)。
/// 角度は度数(`deg`)以外(`rad`/`grad`/`turn`)も含めてパース時点で
/// ラジアンへ正規化する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedTransformFunction {
    Translate(SpecifiedLengthPercentage, SpecifiedLengthPercentage),
    TranslateX(SpecifiedLengthPercentage),
    TranslateY(SpecifiedLengthPercentage),
    Scale(f32, f32),
    ScaleX(f32),
    ScaleY(f32),
    /// ラジアン単位。
    Rotate(f32),
    /// ラジアン単位(水平, 垂直)。
    Skew(f32, f32),
    SkewX(f32),
    SkewY(f32),
    Matrix(f32, f32, f32, f32, f32, f32),
}

impl SpecifiedTransformFunction {
    pub fn resolve(self, font_size: f32, root_font_size: f32) -> TransformFunction {
        match self {
            Self::Translate(x, y) => TransformFunction::Translate(
                x.resolve(font_size, root_font_size),
                y.resolve(font_size, root_font_size),
            ),
            Self::TranslateX(x) => {
                TransformFunction::TranslateX(x.resolve(font_size, root_font_size))
            }
            Self::TranslateY(y) => {
                TransformFunction::TranslateY(y.resolve(font_size, root_font_size))
            }
            Self::Scale(x, y) => TransformFunction::Scale(x, y),
            Self::ScaleX(x) => TransformFunction::ScaleX(x),
            Self::ScaleY(y) => TransformFunction::ScaleY(y),
            Self::Rotate(r) => TransformFunction::Rotate(r),
            Self::Skew(x, y) => TransformFunction::Skew(x, y),
            Self::SkewX(x) => TransformFunction::SkewX(x),
            Self::SkewY(y) => TransformFunction::SkewY(y),
            Self::Matrix(a, b, c, d, e, f) => TransformFunction::Matrix(a, b, c, d, e, f),
        }
    }
}

/// `transform`関数1つ分の計算値。`translate`系のパーセンテージは要素自身の
/// border-box幅/高さが確定してから解決するため`LengthPercentage`のまま
/// 保持する(`background-position`と同じ考え方)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformFunction {
    Translate(LengthPercentage, LengthPercentage),
    TranslateX(LengthPercentage),
    TranslateY(LengthPercentage),
    Scale(f32, f32),
    ScaleX(f32),
    ScaleY(f32),
    Rotate(f32),
    Skew(f32, f32),
    SkewX(f32),
    SkewY(f32),
    Matrix(f32, f32, f32, f32, f32, f32),
}

fn resolve_lp_against(lp: LengthPercentage, basis: f32) -> f32 {
    match lp {
        LengthPercentage::Length(px) => px,
        LengthPercentage::Percentage(p) => p * basis,
        LengthPercentage::Calc { px, percent } => px + percent * basis,
    }
}

impl TransformFunction {
    /// この関数1つ分の変形行列。CSSの`matrix(a, b, c, d, e, f)`と同じ規約
    /// (`x' = a*x + c*y + e`, `y' = b*x + d*y + f`)、CSS座標系(Y下向き正)
    /// のまま返す(PDF座標系への変換は[`compose_transform`]の呼び出し側が
    /// 行う)。`box_width`/`box_height`は`translate`系のパーセンテージ解決に
    /// 使う要素自身のborder-boxサイズ。
    pub fn to_matrix(self, box_width: f32, box_height: f32) -> [f32; 6] {
        match self {
            Self::Translate(x, y) => [
                1.0,
                0.0,
                0.0,
                1.0,
                resolve_lp_against(x, box_width),
                resolve_lp_against(y, box_height),
            ],
            Self::TranslateX(x) => [1.0, 0.0, 0.0, 1.0, resolve_lp_against(x, box_width), 0.0],
            Self::TranslateY(y) => [1.0, 0.0, 0.0, 1.0, 0.0, resolve_lp_against(y, box_height)],
            Self::Scale(sx, sy) => [sx, 0.0, 0.0, sy, 0.0, 0.0],
            Self::ScaleX(sx) => [sx, 0.0, 0.0, 1.0, 0.0, 0.0],
            Self::ScaleY(sy) => [1.0, 0.0, 0.0, sy, 0.0, 0.0],
            Self::Rotate(radians) => {
                let (s, c) = radians.sin_cos();
                [c, s, -s, c, 0.0, 0.0]
            }
            Self::Skew(ax, ay) => [1.0, ay.tan(), ax.tan(), 1.0, 0.0, 0.0],
            Self::SkewX(ax) => [1.0, 0.0, ax.tan(), 1.0, 0.0, 0.0],
            Self::SkewY(ay) => [1.0, ay.tan(), 0.0, 1.0, 0.0, 0.0],
            Self::Matrix(a, b, c, d, e, f) => [a, b, c, d, e, f],
        }
    }
}

/// `a`を先に適用し、その結果へ`b`を適用する合成行列(`b ∘ a`、`matrix(...)`と
/// 同じ規約)。`transform`の複数関数を記述順に合成するために使う。
pub fn compose_transform_matrices(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    let [a1, b1, c1, d1, e1, f1] = b;
    let [a2, b2, c2, d2, e2, f2] = a;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    ]
}

/// `functions`を記述順に合成した1つの変形行列(CSS座標系のまま)。
pub fn compose_transform(
    functions: &[TransformFunction],
    box_width: f32,
    box_height: f32,
) -> [f32; 6] {
    functions
        .iter()
        .fold([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], |acc, f| {
            compose_transform_matrices(acc, f.to_matrix(box_width, box_height))
        })
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    fn assert_matrix_eq(a: [f32; 6], b: [f32; 6]) {
        for i in 0..6 {
            assert!(
                (a[i] - b[i]).abs() < 1e-4,
                "matrices differ at index {i}: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn translate_uses_percentage_against_own_box_size() {
        let m = TransformFunction::Translate(
            LengthPercentage::Percentage(0.5),
            LengthPercentage::Length(10.0),
        )
        .to_matrix(200.0, 50.0);
        assert_matrix_eq(m, [1.0, 0.0, 0.0, 1.0, 100.0, 10.0]);
    }

    #[test]
    fn rotate_90_degrees_matches_the_standard_rotation_matrix() {
        let m = TransformFunction::Rotate(std::f32::consts::FRAC_PI_2).to_matrix(0.0, 0.0);
        assert_matrix_eq(m, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn composing_translate_then_scale_applies_translate_first() {
        // translate(10px, 0) してからscale(2)すると、原点は(10,0)→(20,0)になる
        // (先に平行移動、その結果を拡大するので平行移動量も2倍になる)。
        let translate = TransformFunction::TranslateX(LengthPercentage::Length(10.0));
        let scale = TransformFunction::Scale(2.0, 2.0);
        let total = compose_transform(&[translate, scale], 0.0, 0.0);
        assert_matrix_eq(total, [2.0, 0.0, 0.0, 2.0, 20.0, 0.0]);
    }

    #[test]
    fn identity_matrix_for_empty_function_list() {
        let total = compose_transform(&[], 0.0, 0.0);
        assert_matrix_eq(total, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
}
