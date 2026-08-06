//! `@font-face`ルールから実際のフォントファイルを読み込む。
//!
//! CSSは`<style>`要素からのみ得られ(外部`.css`ファイル・ネットワーク経由の取得は
//! 現状どこにも存在しない)、`src: url(...)`はHTMLファイル自身のディレクトリを
//! 基準とするローカルファイルパスとして解決する。`src: local(...)`はシステム
//! フォントのフルネーム/PostScript名として[`super::system::SystemFonts`]から
//! 解決する。

use cssparser::UnicodeRange;

use crate::img::{ImageFetcher, ImgSrc};
use crate::style::{FontFaceRule, FontFaceSource, FontStyle, FontWeight};

use super::font::Font;
use super::system::SystemFonts;

/// `@font-face`から読み込めたフォントと、CSS側で宣言されたfamily名・weight・style・
/// unicode-range。
pub struct LoadedFontFace {
    pub family: String,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub unicode_range: Vec<UnicodeRange>,
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
    fetcher: &ImageFetcher,
    system: &SystemFonts,
) -> Vec<LoadedFontFace> {
    font_faces
        .iter()
        .filter_map(|rule| load_one(rule, fetcher, system))
        .collect()
}

fn load_one(
    rule: &FontFaceRule,
    fetcher: &ImageFetcher,
    system: &SystemFonts,
) -> Option<LoadedFontFace> {
    for src in &rule.src {
        let font = match src {
            // 読み込みは`<img>`・`<link>`・`@import`と同じ[`ImageFetcher`]を
            // 通す。直接`fs::read`すると、ローカルアクセスの可否
            // (`--disable-local-file-access`)・許可ディレクトリ(`--allow`)・
            // サイズ上限のいずれも効かない抜け道になる。
            //
            // T61: root-relativeな`url("/fonts/brand.ttf")`をbase_dir相対と
            // して扱う挙動は、フェッチャ側の`resolve_local_asset_path`が
            // 引き続き担う。
            FontFaceSource::Url(path) => fetcher
                .fetch(&ImgSrc::LocalPath(path.clone()))
                .ok()
                .and_then(|bytes| Font::from_bytes(bytes, 0).ok()),
            FontFaceSource::Local(name) => system.load_by_full_name(name),
        };
        if let Some(font) = font {
            return Some(LoadedFontFace {
                family: rule.family.clone(),
                weight: rule.weight,
                style: rule.style,
                unicode_range: rule.unicode_range.clone(),
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
    use std::path::Path;

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");

    /// テスト用のフェッチャ。既定はCLIと同じ「ローカル読み込み可・許可
    /// ディレクトリの限定なし」。
    fn fetcher() -> ImageFetcher {
        ImageFetcher::new(Path::new(DEJAVU_PATH).to_path_buf(), false)
    }

    fn rule(family: &str, src: Vec<FontFaceSource>) -> FontFaceRule {
        FontFaceRule {
            family: family.to_string(),
            src,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
            unicode_range: Vec::new(),
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
        let loaded = load_font_faces(&rules, &fetcher(), &no_system_fonts());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn resolves_a_root_relative_url_within_base_dir() {
        // T61: `url("/DejaVuSans.ttf")`のようなroot-relativeな書き方も
        // base_dir配下のファイルとして解決されるはず(OSのファイルシステム
        // ルートへ逃げない)。
        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Url("/DejaVuSans.ttf".to_string())],
        )];
        let loaded = load_font_faces(&rules, &fetcher(), &no_system_fonts());

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
        let loaded = load_font_faces(&rules, &fetcher(), &no_system_fonts());

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    #[test]
    fn skips_a_font_face_rule_whose_sources_are_all_unusable() {
        let rules = vec![rule(
            "Missing Brand",
            vec![FontFaceSource::Url("does-not-exist.ttf".to_string())],
        )];
        let loaded = load_font_faces(&rules, &fetcher(), &no_system_fonts());

        assert!(loaded.is_empty());
    }

    #[test]
    fn resolves_local_source_from_the_system_font_database() {
        let system = SystemFonts::from_dir(Path::new(DEJAVU_PATH));
        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Local("DejaVu Sans".to_string())],
        )];
        let loaded = load_font_faces(&rules, &fetcher(), &system);

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }

    /// `--disable-local-file-access`が`@font-face`の`url()`にも効くこと。
    /// 以前はここだけフェッチャを通さず直接`fs::read`していたため、
    /// ローカルアクセスを禁止していても読めてしまっていた。
    #[test]
    fn a_url_source_is_refused_when_local_file_access_is_disabled() {
        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Url("DejaVuSans.ttf".to_string())],
        )];
        let blocked = ImageFetcher::new(Path::new(DEJAVU_PATH).to_path_buf(), false)
            .with_local_access(false, Vec::new());

        let loaded = load_font_faces(&rules, &blocked, &no_system_fonts());
        assert!(
            loaded.is_empty(),
            "ローカル読み込みを禁止したらurl()のフォントも読めてはならない"
        );
    }

    /// `--allow`で許可したディレクトリの外にあるフォントは、`..`で辿っても
    /// 読めないこと。
    #[test]
    fn a_url_source_outside_the_allowed_dirs_is_refused() {
        let base = Path::new(DEJAVU_PATH).to_path_buf();
        // base_dir自身は許可せず、その下の存在しないサブディレクトリだけ許可する。
        let allowed = vec![base.join("allowed-subdir")];
        let restricted = ImageFetcher::new(base, false).with_local_access(true, allowed);

        let rules = vec![rule(
            "Custom Brand",
            vec![FontFaceSource::Url("DejaVuSans.ttf".to_string())],
        )];
        let loaded = load_font_faces(&rules, &restricted, &no_system_fonts());
        assert!(
            loaded.is_empty(),
            "--allowの範囲外にあるフォントは読めてはならない"
        );
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
        let loaded = load_font_faces(&rules, &fetcher(), &system);

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].family, "Custom Brand");
    }
}
