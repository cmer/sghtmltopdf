//! CSSプロパティ値の型。M1で対応する最小セットのみ。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    None,
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
