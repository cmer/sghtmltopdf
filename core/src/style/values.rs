//! CSSプロパティ値の型。M1で対応する最小セットのみ。

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

/// `border-style`。M1では実線・破線・点線のみ対応
/// (`double`/`groove`/`ridge`/`inset`/`outset`は非対応)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
}

/// 長さ(px)またはパーセンテージ。M1では単位はpxのみ対応する。
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
