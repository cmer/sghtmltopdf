//! `@font-face`ルールから実際のフォントファイルを読み込む。
//!
//! CSSは`<style>`要素からのみ得られ(外部`.css`ファイル・ネットワーク経由の取得は
//! 現状どこにも存在しない)、`src: url(...)`はHTMLファイル自身のディレクトリを
//! 基準とするローカルファイルパスとして解決する。`src: local(...)`はシステム
//! フォントのフルネーム/PostScript名として[`super::system::SystemFonts`]から
//! 解決する。

use std::path::Path;

use crate::style::{FontFaceRule, FontFaceSource, FontStyle, FontWeight};

use super::font::Font;
use super::system::SystemFonts;

/// `@font-face`から読み込めたフォントと、CSS側で宣言されたfamily名・weight・style。
pub struct LoadedFontFace {
    pub family: String,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub font: Font,
}

/// `font_faces`それぞれについて、`src`に列挙された`url(...)`/`local(...)`を
/// 先頭から順に試し、最初に読み込めたものを採用する(`format()`ヒントは検証
/// しないため、非対応フォーマット(WOFF/WOFF2等)は単にパース失敗として次の
/// 候補に読み進める)。どの`src`も読み込めなかった`@font-face`ルールは標準
/// エラー出力に警告を出して無視する(1つのフォントの欠落のために変換全体を
/// 失敗させない)。
pub fn load_font_faces(
    font_faces: &[FontFaceRule],
    base_dir: &Path,
    system: &SystemFonts,
) -> Vec<LoadedFontFace> {
    font_faces
        .iter()
        .filter_map(|rule| load_one(rule, base_dir, system))
        .collect()
}

fn load_one(rule: &FontFaceRule, base_dir: &Path, system: &SystemFonts) -> Option<LoadedFontFace> {
    for src in &rule.src {
        let font = match src {
            FontFaceSource::Url(path) => std::fs::read(base_dir.join(path))
                .ok()
                .and_then(|bytes| Font::from_bytes(bytes, 0).ok()),
            FontFaceSource::Local(name) => system.load_by_full_name(name),
        };
        if let Some(font) = font {
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

    fn rule(family: &str, src: Vec<FontFaceSource>) -> FontFaceRule {
        FontFaceRule {
            family: family.to_string(),
            src,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        }
    }

    fn no_system_fonts() -> SystemFonts {
        // ローカルの空ディレクトリを走査させ、システムフォントが1つも
        // 無い状態を作る(local()解決の対象外テスト用)。
        SystemFonts::from_dir(Path::new(DEJAVU_PATH).join("does-not-exist").as_path())
    }

    #[test]
    fn loads_a_font_from_a_relative_url() {
        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Url("DejaVuSans.ttf".to_string())],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH), &no_system_fonts());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn falls_through_to_the_next_src_when_the_first_one_is_missing() {
        let rules = vec![rule(
            "Custom Brand",
            vec![
                FontFaceSource::Url("does-not-exist.ttf".to_string()),
                FontFaceSource::Url("DejaVuSans.ttf".to_string()),
            ],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH), &no_system_fonts());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn skips_a_font_face_rule_whose_sources_are_all_unusable() {
        let rules = vec![rule(
            "Missing Brand",
            vec![FontFaceSource::Url("does-not-exist.ttf".to_string())],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH), &no_system_fonts());

        assert!(loaded.is_empty());
    }

    #[test]
    fn resolves_local_source_from_the_system_font_database() {
        let system = SystemFonts::from_dir(Path::new(DEJAVU_PATH));
        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Local("DejaVu Sans".to_string())],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH), &system);

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn falls_through_from_an_unresolvable_local_source_to_a_url() {
        let system = SystemFonts::from_dir(Path::new(DEJAVU_PATH));
        let rules = vec![rule(
            "Custom Brand",
            vec![
                FontFaceSource::Local("Definitely Not Installed".to_string()),
                FontFaceSource::Url("DejaVuSans.ttf".to_string()),
            ],
        )];
        let loaded = load_font_faces(&rules, Path::new(DEJAVU_PATH), &system);

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }
}
