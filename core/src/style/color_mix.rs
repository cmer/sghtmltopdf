//! `color-mix()`の混色計算(CSS Color 5 §3、補間の規則はCSS Color 4 §12)。
//!
//! 構文の読み取りは[`super::properties`]が行い、ここは「色空間・色相の回し方・
//! 2色と重み」を受け取って混ぜるところだけを持つ。
//!
//! 出力先がPDFのDeviceRGBなので、結果は常にsRGBへ戻して返す。

use palette::{FromColor, Hsl, Hwb, Lab, LinSrgb, Oklab, Oklch, Srgb, Xyz};

/// 補間に使う色空間。
///
/// `display-p3`/`a98-rgb`/`prophoto-rgb`/`rec2020`は非対応。sRGBより広い
/// 色域を扱えないため、受け付けても結果はsRGBへ丸められるだけで、指定した
/// 意味にならない。仕様どおり無効な`<color-space>`として扱い、宣言ごと落とす。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Space {
    Srgb,
    SrgbLinear,
    Lab,
    Oklab,
    Xyz,
    Hsl,
    Hwb,
    Lch,
    Oklch,
}

impl Space {
    pub(super) fn parse(ident: &str) -> Option<Self> {
        Some(match ident.to_ascii_lowercase().as_str() {
            "srgb" => Self::Srgb,
            "srgb-linear" => Self::SrgbLinear,
            "lab" => Self::Lab,
            "oklab" => Self::Oklab,
            // CSSの`xyz`は`xyz-d65`の別名。
            "xyz" | "xyz-d65" => Self::Xyz,
            "hsl" => Self::Hsl,
            "hwb" => Self::Hwb,
            "lch" => Self::Lch,
            "oklch" => Self::Oklch,
            _ => return None,
        })
    }

    /// 色相を持つ(極座標)色空間か。持つ場合は成分列の何番目が色相かを返す。
    fn hue_index(self) -> Option<usize> {
        match self {
            // paletteの`Hsl`/`Hwb`は色相が先頭。
            Self::Hsl | Self::Hwb => Some(0),
            // `Lch`/`Oklch`は明度・彩度の後ろ。
            Self::Lch | Self::Oklch => Some(2),
            _ => None,
        }
    }

    /// sRGBから、この色空間の成分列(色相は度)へ。
    fn components_of(self, c: Srgb) -> [f32; 3] {
        match self {
            Self::Srgb => [c.red, c.green, c.blue],
            Self::SrgbLinear => {
                let v = LinSrgb::from_color(c);
                [v.red, v.green, v.blue]
            }
            Self::Lab => {
                let v = Lab::from_color(c);
                [v.l, v.a, v.b]
            }
            Self::Oklab => {
                let v = Oklab::from_color(c);
                [v.l, v.a, v.b]
            }
            Self::Xyz => {
                let v = Xyz::from_color(c);
                [v.x, v.y, v.z]
            }
            Self::Hsl => {
                let v = Hsl::from_color(c);
                [v.hue.into_degrees(), v.saturation, v.lightness]
            }
            Self::Hwb => {
                let v = Hwb::from_color(c);
                [v.hue.into_degrees(), v.whiteness, v.blackness]
            }
            Self::Lch => {
                let v = palette::Lch::from_color(c);
                [v.l, v.chroma, v.hue.into_degrees()]
            }
            Self::Oklch => {
                let v = Oklch::from_color(c);
                [v.l, v.chroma, v.hue.into_degrees()]
            }
        }
    }

    /// この色空間の成分列からsRGBへ。
    fn to_srgb(self, v: [f32; 3]) -> Srgb {
        match self {
            Self::Srgb => Srgb::new(v[0], v[1], v[2]),
            Self::SrgbLinear => Srgb::from_color(LinSrgb::new(v[0], v[1], v[2])),
            Self::Lab => Srgb::from_color(Lab::new(v[0], v[1], v[2])),
            Self::Oklab => Srgb::from_color(Oklab::new(v[0], v[1], v[2])),
            Self::Xyz => Srgb::from_color(Xyz::new(v[0], v[1], v[2])),
            Self::Hsl => Srgb::from_color(Hsl::new(v[0], v[1], v[2])),
            Self::Hwb => Srgb::from_color(Hwb::new(v[0], v[1], v[2])),
            Self::Lch => Srgb::from_color(palette::Lch::new(v[0], v[1], v[2])),
            Self::Oklch => Srgb::from_color(Oklch::new(v[0], v[1], v[2])),
        }
    }
}

/// 色相をどちら回りで補間するか(CSS Color 4 §12.4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum HueMethod {
    /// 初期値。短い方の弧を通る。
    #[default]
    Shorter,
    /// 長い方の弧を通る。
    Longer,
    /// 色相が増える向きに回る。
    Increasing,
    /// 色相が減る向きに回る。
    Decreasing,
}

impl HueMethod {
    pub(super) fn parse(ident: &str) -> Option<Self> {
        Some(match ident.to_ascii_lowercase().as_str() {
            "shorter" => Self::Shorter,
            "longer" => Self::Longer,
            "increasing" => Self::Increasing,
            "decreasing" => Self::Decreasing,
            _ => return None,
        })
    }

    /// 補間前に色相の組を調整する。返り値をそのまま線形補間すると、
    /// 指定した回り方になる。
    fn adjust(self, h1: f32, h2: f32) -> (f32, f32) {
        let (h1, h2) = (normalize_hue(h1), normalize_hue(h2));
        let diff = h2 - h1;
        match self {
            Self::Shorter => {
                if diff > 180.0 {
                    (h1 + 360.0, h2)
                } else if diff < -180.0 {
                    (h1, h2 + 360.0)
                } else {
                    (h1, h2)
                }
            }
            Self::Longer => {
                if (0.0..180.0).contains(&diff) {
                    (h1 + 360.0, h2)
                } else if diff > -180.0 && diff <= 0.0 {
                    (h1, h2 + 360.0)
                } else {
                    (h1, h2)
                }
            }
            Self::Increasing => {
                if h2 < h1 {
                    (h1, h2 + 360.0)
                } else {
                    (h1, h2)
                }
            }
            Self::Decreasing => {
                if h1 < h2 {
                    (h1 + 360.0, h2)
                } else {
                    (h1, h2)
                }
            }
        }
    }
}

/// 度を`[0, 360)`へ収める。
fn normalize_hue(h: f32) -> f32 {
    let h = h % 360.0;
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

/// sRGBの0.0〜1.0で表した色とアルファ。
pub(super) type UnitRgba = (f32, f32, f32, f32);

/// `space`で2色を混ぜ、sRGBの0.0〜1.0で返す。`w1`+`w2`は1.0であること
/// (重みの正規化は呼び出し側が済ませる)。
///
/// アルファは事前乗算してから補間する(CSS Color 4 §12.3)。色相だけは
/// 乗算の対象外。
pub(super) fn mix(
    space: Space,
    hue: HueMethod,
    c1: UnitRgba,
    w1: f32,
    c2: UnitRgba,
    w2: f32,
) -> UnitRgba {
    let alpha = c1.3 * w1 + c2.3 * w2;

    let mut v1 = space.components_of(Srgb::new(c1.0, c1.1, c1.2));
    let mut v2 = space.components_of(Srgb::new(c2.0, c2.1, c2.2));

    let hue_index = space.hue_index();
    if let Some(i) = hue_index {
        let (h1, h2) = hue.adjust(v1[i], v2[i]);
        v1[i] = h1;
        v2[i] = h2;
    }
    for i in 0..3 {
        if Some(i) == hue_index {
            continue;
        }
        v1[i] *= c1.3;
        v2[i] *= c2.3;
    }

    let mut mixed = [0.0f32; 3];
    for i in 0..3 {
        mixed[i] = v1[i] * w1 + v2[i] * w2;
        if Some(i) != hue_index && alpha != 0.0 {
            // 事前乗算を戻す。
            mixed[i] /= alpha;
        }
    }

    let srgb = space.to_srgb(mixed);
    (srgb.red, srgb.green, srgb.blue, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: UnitRgba = (1.0, 0.0, 0.0, 1.0);
    const BLUE: UnitRgba = (0.0, 0.0, 1.0, 1.0);

    fn to_u8(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn mixed_rgb(space: Space, hue: HueMethod, a: UnitRgba, b: UnitRgba) -> (u8, u8, u8) {
        let (r, g, bl, _) = mix(space, hue, a, 0.5, b, 0.5);
        (to_u8(r), to_u8(g), to_u8(bl))
    }

    #[test]
    fn srgb_midpoint_is_the_arithmetic_mean() {
        assert_eq!(
            mixed_rgb(Space::Srgb, HueMethod::Shorter, RED, BLUE),
            (128, 0, 128)
        );
    }

    /// `hsl`は色相環を回るので、赤(0度)と青(240度)の中間は短い方の弧を通って
    /// 300度(マゼンタ)になる。算術平均の120度(緑)にはならない。
    #[test]
    fn a_polar_space_takes_the_shorter_arc_by_default() {
        assert_eq!(
            mixed_rgb(Space::Hsl, HueMethod::Shorter, RED, BLUE),
            (255, 0, 255)
        );
    }

    /// `longer hue`なら反対回りで、中間は120度(緑)になる。
    #[test]
    fn longer_hue_takes_the_other_arc() {
        assert_eq!(
            mixed_rgb(Space::Hsl, HueMethod::Longer, RED, BLUE),
            (0, 255, 0)
        );
    }

    #[test]
    fn increasing_and_decreasing_hue_pick_a_direction() {
        // 0度から240度へ増える向き = 120度(緑)。
        assert_eq!(
            mixed_rgb(Space::Hsl, HueMethod::Increasing, RED, BLUE),
            (0, 255, 0)
        );
        // 減る向き = 300度(マゼンタ)。
        assert_eq!(
            mixed_rgb(Space::Hsl, HueMethod::Decreasing, RED, BLUE),
            (255, 0, 255)
        );
    }

    /// アルファが違う色を混ぜるときは事前乗算する。透明度の高い方の色味が
    /// 薄く出るのが正しい(単純平均だと濃く出すぎる)。
    #[test]
    fn alpha_is_premultiplied_before_interpolating() {
        let half_red = (1.0, 0.0, 0.0, 0.5);
        let (r, g, b, a) = mix(Space::Srgb, HueMethod::Shorter, half_red, 0.5, BLUE, 0.5);
        assert_eq!((to_u8(r), to_u8(g), to_u8(b)), (85, 0, 170));
        assert!((a - 0.75).abs() < 1e-6, "alpha={a}");
    }

    #[test]
    fn lab_midpoint_of_white_and_black_is_perceptual_grey() {
        let white = (1.0, 1.0, 1.0, 1.0);
        let black = (0.0, 0.0, 0.0, 1.0);
        // L=50の灰色。sRGBの算術平均(128)より暗い。
        assert_eq!(
            mixed_rgb(Space::Lab, HueMethod::Shorter, white, black),
            (119, 119, 119)
        );
    }

    #[test]
    fn unknown_color_spaces_are_rejected() {
        assert!(Space::parse("display-p3").is_none());
        assert!(Space::parse("rec2020").is_none());
        assert_eq!(Space::parse("OKLCH"), Some(Space::Oklch));
        assert_eq!(Space::parse("xyz-d65"), Some(Space::Xyz));
    }
}
