//! `@font-face`ルールから実際のフォントファイルを読み込む。
//!
//! CSSは`<style>`要素からのみ得られ(外部`.css`ファイル・ネットワーク経由の取得は
//! 現状どこにも存在しない)、`src: url(...)`はHTMLファイル自身のディレクトリを
//! 基準とするローカルファイルパスとして解決する。

use std::path::Path;

use crate::style::{FontFaceRule, FontStyle, FontWeight};

use super::font::Font;

/// `@font-face`から読み込めたフォントと、CSS側で宣言されたfamily名・weight・style。
pub struct LoadedFontFace {
    pub family: String,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub font: Font,
}

/// `font_faces`それぞれについて、`src`に列挙された`url(...)`を先頭から順に試し、
/// `base_dir`基準の相対パスとして読み込める最初の1つを採用する
/// (`format()`ヒントは検証しないため、非対応フォーマット(WOFF/WOFF2等)は
/// 単にパース失敗として次の候補に読み進める)。どの`src`も読み込めなかった
/// `@font-face`ルールは標準エラー出力に警告を出して無視する(1つのフォントの
/// 欠落のために変換全体を失敗させない)。
pub fn load_font_faces(font_faces: &[FontFaceRule], base_dir: &Path) -> Vec<LoadedFontFace> {
    font_faces
        .iter()
        .filter_map(|rule| load_one(rule, base_dir))
        .collect()
}

fn load_one(rule: &FontFaceRule, base_dir: &Path) -> Option<LoadedFontFace> {
    for src in &rule.src {
        let path = base_dir.join(src);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(font) = Font::from_bytes(bytes, 0) {
            return Some(LoadedFontFace {
                family: rule.family.clone(),
                weight: rule.weight,
                style: rule.style,
                font,
            });
        }
    }
    eprintln!(
        "警告: @font-face \"{}\"の読み込みに失敗しました(有効なsrcが見つかりません)",
        rule.family
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{FontStyle, FontWeight};

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");

    fn rule(family: &str, src: Vec<String>) -> FontFaceRule {
        FontFaceRule {
            family: family.to_string(),
            src,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        }
    }

    #[test]
    fn loads_a_font_from_a_relative_url() {
        let rules = vec![rule("Custom Brand", vec!["DejaVuSans.ttf".to_string()])];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH));

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn falls_through_to_the_next_src_when_the_first_one_is_missing() {
        let rules = vec![rule(
            "Custom Brand",
            vec![
                "does-not-exist.ttf".to_string(),
                "DejaVuSans.ttf".to_string(),
            ],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH));

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn skips_a_font_face_rule_whose_sources_are_all_unusable() {
        let rules = vec![rule(
            "Missing Brand",
            vec!["does-not-exist.ttf".to_string()],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH));

        assert!(loaded.is_empty());
    }
}
