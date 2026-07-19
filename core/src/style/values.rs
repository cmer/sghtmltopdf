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
    None,
}

/// `font-weight`。数値指定(`700`等)は600以上を`Bold`として扱う簡略実装。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/// `font-style`。`oblique`は専用の傾斜を持たないため`Italic`と同一視する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
