//! OS標準のフォントディレクトリを走査してシステムフォントを解決する(`fontdb`を使用)。
//!
//! CSSの汎用family名(`sans-serif`/`serif`等)はここでは解決しない対象とする。
//! `fontdb`はLinuxではfontconfigの設定(`/etc/fonts/fonts.conf`)から汎用名の
//! 実体を拾えるが、fontconfig未設置の最小環境ではOS間で一貫性のないハードコードの
//! 既定名(`Arial`等)にフォールバックし、インストール環境に実在するとは限らない。
//! [`crate::fonts::FontCollection`]が既に持つグリフカバレッジ・フォールバックに
//! 任せ、ここでは`font-family`で名指しされた具体的なフォント名のみを対象にする。

use std::collections::HashMap;
use std::collections::HashSet;

use crate::html::NodeId;
use crate::style::{ComputedStyle, FontStyle, FontWeight};

use super::collection::FontCollection;
use super::font::Font;

/// CSSの汎用family名(大文字小文字を区別せず判定する)。
const GENERIC_FAMILIES: &[&str] = &["serif", "sans-serif", "monospace", "cursive", "fantasy"];

pub struct SystemFonts {
    db: fontdb::Database,
}

impl SystemFonts {
    /// OSのフォントディレクトリを走査してデータベースを構築する
    /// (メタデータのスキャンのみ。フォントファイルの実体はまだ読み込まない)。
    pub fn scan() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self { db }
    }

    #[cfg(test)]
    fn from_dir(dir: &std::path::Path) -> Self {
        let mut db = fontdb::Database::new();
        db.load_fonts_dir(dir);
        Self { db }
    }

    /// `family`という名前のシステムフォントを読み込む(大文字小文字を区別しない)。
    /// `weight`/`style`は`fontdb`のCSSライクなマッチングにそのまま渡すため、
    /// 例えば`weight: Bold`で該当familyに本物のBold面が存在すればそれが選ばれる
    /// (存在しなければ`fontdb`が代わりに最も近い面を返し、その場合は呼び出し側が
    /// `FontCollection::is_bold`等で実体を確認した上で疑似太字を補うことになる)。
    /// 一致するフォントが無ければ`None`。
    pub fn load(&self, family: &str, weight: FontWeight, style: FontStyle) -> Option<Font> {
        // `fontdb::Database::query`はfamily名の完全一致(大文字小文字を区別する)
        // でしか照合しないため、まず大文字小文字を無視して実際の登録名を探し、
        // その名前で改めてクエリする。
        let exact_name = self
            .db
            .faces()
            .flat_map(|info| info.families.iter())
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
            .map(|(name, _)| name.clone())?;

        let query = fontdb::Query {
            families: &[fontdb::Family::Name(&exact_name)],
            weight: to_fontdb_weight(weight),
            style: to_fontdb_style(style),
            ..Default::default()
        };
        let id = self.db.query(&query)?;
        self.db
            .with_face_data(id, |data, index| {
                Font::from_bytes(data.to_vec(), index).ok()
            })
            .flatten()
    }
}

fn to_fontdb_weight(weight: FontWeight) -> fontdb::Weight {
    match weight {
        FontWeight::Normal => fontdb::Weight::NORMAL,
        FontWeight::Bold => fontdb::Weight::BOLD,
    }
}

fn to_fontdb_style(style: FontStyle) -> fontdb::Style {
    match style {
        FontStyle::Normal => fontdb::Style::Normal,
        FontStyle::Italic => fontdb::Style::Italic,
    }
}

/// `styles`中で使われている具体的な(CSS汎用キーワードではない)
/// font-family/weight/styleの組のうち、`fonts`にまだ実体が無いものだけを
/// `system`から読み込み、`fonts`へ追加する。
///
/// `family`単位ではなく(family, weight, style)単位で判定するため、例えば
/// `--font`でRegularのみ読み込んだfamilyに対して文書内で太字が使われていれば、
/// そのfamilyのBold面だけを追加でシステムから探しに行く。
pub fn load_missing_system_fonts(
    fonts: &mut FontCollection,
    styles: &HashMap<NodeId, ComputedStyle>,
    system: &SystemFonts,
) {
    let mut seen = HashSet::new();
    for style in styles.values() {
        for family in &style.font_family {
            let key = (family.clone(), style.font_weight, style.font_style);
            if !seen.insert(key) {
                continue;
            }
            if GENERIC_FAMILIES
                .iter()
                .any(|g| g.eq_ignore_ascii_case(family))
            {
                continue;
            }
            if fonts.has_matching_face(family, style.font_weight, style.font_style) {
                continue;
            }
            if let Some(font) = system.load(family, style.font_weight, style.font_style) {
                fonts.push_font_face(family.clone(), None, None, font);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const FONTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts");

    #[test]
    fn loads_a_font_by_family_name_case_insensitively() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let font = system
            .load("dejavu sans", FontWeight::Normal, FontStyle::Normal)
            .expect("should find DejaVu Sans regardless of case");
        assert!(font.has_glyph('A'));
    }

    #[test]
    fn returns_none_for_an_unknown_family() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        assert!(system
            .load(
                "Definitely Not A Real Font",
                FontWeight::Normal,
                FontStyle::Normal
            )
            .is_none());
    }

    #[test]
    fn loads_the_real_bold_face_when_the_family_has_one() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let font = system
            .load("DejaVu Sans", FontWeight::Bold, FontStyle::Normal)
            .expect("should find a DejaVu Sans face");
        assert!(
            font.weight() >= 600,
            "should resolve to the real bold face (DejaVuSans-Bold.ttf), not the regular one"
        );
    }

    #[test]
    fn load_missing_system_fonts_adds_fonts_used_by_the_document() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let dom = html::parse(br#"<p style="font-family: 'DejaVu Sans';">text</p>"#);
        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());

        let mut fonts = FontCollection::new(vec![]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert_eq!(fonts.len(), 1);
        assert!(fonts.has_family("DejaVu Sans"));
    }

    #[test]
    fn load_missing_system_fonts_skips_families_already_present() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let dom = html::parse(br#"<p style="font-family: 'DejaVu Sans';">text</p>"#);
        let styles = compute_styles(&dom, &Stylesheet::default(), &Stylesheet::default());

        let mut fonts = FontCollection::new(vec![Font::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fonts/DejaVuSans.ttf"
        ))
        .unwrap()]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert_eq!(
            fonts.len(),
            1,
            "already-loaded family should not be duplicated"
        );
    }

    #[test]
    fn load_missing_system_fonts_still_searches_for_a_missing_weight_of_a_known_family() {
        // `--font`でRegularのDejaVu Sansのみ読み込み済みの状態で、文書は同じ
        // familyのBold(<b>)も使う。family自体は既に存在するが、Bold面は
        // まだ無いので、そのweightだけを追加でシステムから探しに行くはず。
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let dom = html::parse(br#"<p style="font-family: 'DejaVu Sans';">a <b>b</b></p>"#);
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &Stylesheet::default());

        let mut fonts = FontCollection::new(vec![Font::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fonts/DejaVuSans.ttf"
        ))
        .unwrap()]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert_eq!(
            fonts.len(),
            2,
            "the missing bold face should be added alongside the existing regular one"
        );
        assert!(
            fonts.has_matching_face("DejaVu Sans", FontWeight::Bold, FontStyle::Normal),
            "a real bold face should now be available for DejaVu Sans"
        );
    }

    #[test]
    fn load_missing_system_fonts_ignores_generic_css_keywords() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let author = parse_stylesheet("p { font-family: sans-serif; }");
        let dom = html::parse(b"<p>text</p>");
        let styles = compute_styles(&dom, &Stylesheet::default(), &author);

        let mut fonts = FontCollection::new(vec![]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert!(
            fonts.is_empty(),
            "generic family keywords should not trigger a system font lookup"
        );
    }
}
