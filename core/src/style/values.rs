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

/// `border-style`。M1では実線・破線・点線・二重線のみ対応。
/// `groove`/`ridge`/`inset`/`outset`(border-colorから2階調の疑似立体陰影を
/// 算出する必要がある)は、請求書・帳票用途での実用性に対して実装コストが
/// 見合わないため非対応とする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
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

#[derive(Debug, Clone, Copy, PartialEq)]
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
