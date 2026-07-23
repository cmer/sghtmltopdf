//! OS標準のフォントディレクトリを走査してシステムフォントを解決する(`fontdb`を使用)。
//!
//! CSSの汎用family名(`monospace`/`serif`)は、`fontdb`(=Linuxでは
//! fontconfig)の汎用名解決には**任せず**、自前の候補リストで解決する
//! ([0036](../../../docs/decisions/0036-ua-stylesheet-and-hidden-elements-design.md)
//! 決定3)。`fontdb`はfontconfig未設置の最小環境ではOS間で一貫性のない
//! ハードコードの既定名(`Arial`等)にフォールバックし、インストール環境に
//! 実在するとは限らないため。候補リストが全て外れた場合、`monospace`のみ
//! `fontdb`のフェース単位のメタデータ(`FaceInfo::monospaced`。fontconfig
//! 非依存)を使って等幅フェースを探す。それでも見つからなければ解決を諦め、
//! [`crate::fonts::FontCollection`]が既に持つグリフカバレッジ・フォールバックに
//! 任せる。`sans-serif`は`ComputedStyle`の既定`font-family`と同値であり、
//! 解決すると`--font`/`@font-face`で明示指定したフォントが既定フォントで
//! なくなってしまうため、意図的に解決対象から外している(決定3-1)。

use std::collections::HashMap;
use std::collections::HashSet;

use crate::html::NodeId;
use crate::style::{ComputedStyle, FontStyle, FontWeight};

use super::collection::FontCollection;
use super::font::Font;

/// CSSの汎用family名(大文字小文字を区別せず判定する)。
const GENERIC_FAMILIES: &[&str] = &["serif", "sans-serif", "monospace", "cursive", "fantasy"];

/// 汎用family名ごとの、実在しやすい具体フォント名の候補(優先順)。
///
/// `cursive`/`fantasy`は環境差が大きく実務上の需要も薄いため候補を持たない
/// (=解決しない、[0036]決定3)。**`sans-serif`も意図的に解決しない**
/// ([0036]決定3-1): `ComputedStyle`の既定`font-family`が`sans-serif`で
/// あるため、これを解決すると「`--font`/`@font-face`で明示的に与えた
/// フォントが既定フォントになる」という挙動が壊れ、PDFに埋め込まれる
/// フォントが実行環境のインストール状況に依存してしまう。
const GENERIC_FAMILY_CANDIDATES: &[(&str, &[&str])] = &[
    (
        "monospace",
        &[
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
            "Ubuntu Mono",
            "Menlo",
            "Consolas",
            "Courier New",
            "Courier",
        ],
    ),
    (
        "serif",
        &[
            "DejaVu Serif",
            "Liberation Serif",
            "Noto Serif",
            "Times New Roman",
            "Times",
            "Georgia",
        ],
    ),
];

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
    pub(super) fn from_dir(dir: &std::path::Path) -> Self {
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

    /// CSSの汎用family名(`monospace`/`serif`)を、自前の候補リスト
    /// ([`GENERIC_FAMILY_CANDIDATES`])を優先順に試して具体フォントへ
    /// 解決する([0036](
    /// ../../../docs/decisions/0036-ua-stylesheet-and-hidden-elements-design.md)
    /// 決定3)。候補が全て外れた場合、`monospace`のみ`fontdb`の
    /// `FaceInfo::monospaced`フラグで等幅フェースを探す。`sans-serif`
    /// (既定`font-family`と同値のため意図的に対象外、決定3-1)・
    /// `cursive`/`fantasy`・汎用名でない名前を渡した場合、および何も
    /// 見つからない場合は`None`。
    pub fn load_generic(
        &self,
        generic: &str,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<Font> {
        let candidates = GENERIC_FAMILY_CANDIDATES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(generic))
            .map(|(_, candidates)| *candidates)?;

        for candidate in candidates {
            if let Some(font) = self.load(candidate, weight, style) {
                return Some(font);
            }
        }

        if generic.eq_ignore_ascii_case("monospace") {
            return self.load_any_monospaced(weight, style);
        }
        None
    }

    /// フォント自身のメタデータ上「等幅」とされているフェースを1つ選び、その
    /// family名で改めて`load`する(weight/styleの面選択を`load`に任せるため)。
    fn load_any_monospaced(&self, weight: FontWeight, style: FontStyle) -> Option<Font> {
        let family = self
            .db
            .faces()
            .filter(|info| info.monospaced)
            .find_map(|info| info.families.first().map(|(name, _)| name.clone()))?;
        self.load(&family, weight, style)
    }

    /// `@font-face`の`src: local(...)`用。`name`(フルネームまたはPostScript名、
    /// 大文字小文字を区別しない)に一致する特定の面を1つ直接読み込む。
    /// `load`(family名+weight/styleによるCSS的なフォールバック検索)とは異なり、
    /// weight/styleによる曖昧なマッチングは行わない(名前で一意に指定された
    /// 1つの面を指すのが`local()`の意味のため)。
    pub fn load_by_full_name(&self, name: &str) -> Option<Font> {
        let info = self.db.faces().find(|info| {
            info.post_script_name.eq_ignore_ascii_case(name)
                || info
                    .families
                    .iter()
                    .any(|(family_name, _)| family_name.eq_ignore_ascii_case(name))
        })?;
        self.db
            .with_face_data(info.id, |data, index| {
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

/// `styles`中で使われているfont-family/weight/styleの組のうち、`fonts`に
/// まだ実体が無いものだけを`system`から読み込み、`fonts`へ追加する。
///
/// `family`単位ではなく(family, weight, style)単位で判定するため、例えば
/// `--font`でRegularのみ読み込んだfamilyに対して文書内で太字が使われていれば、
/// そのfamilyのBold面だけを追加でシステムから探しに行く。
///
/// CSSの汎用family名(`monospace`等)は[`SystemFonts::load_generic`]で解決し、
/// **汎用名そのものを宣言family名として**`fonts`へ登録する
/// ([0036](../../../docs/decisions/0036-ua-stylesheet-and-hidden-elements-design.md)
/// 決定3)。こうすることで`font-family: monospace`の照合が
/// [`FontCollection::select_for_char`]の通常のfamily一致でそのまま機能する。
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
            if fonts.has_matching_face(family, style.font_weight, style.font_style) {
                continue;
            }
            let is_generic = GENERIC_FAMILIES
                .iter()
                .any(|g| g.eq_ignore_ascii_case(family));
            let font = if is_generic {
                system.load_generic(family, style.font_weight, style.font_style)
            } else {
                system.load(family, style.font_weight, style.font_style)
            };
            if let Some(font) = font {
                fonts.push_font_face(family.clone(), None, None, Vec::new(), font);
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
    fn load_missing_system_fonts_does_not_resolve_sans_serif() {
        // `sans-serif`は`ComputedStyle`の既定`font-family`と同値なので、
        // これを解決すると`--font`で渡したフォントが既定フォントでなくなる
        // ([0036]決定3-1)。fixtureには"DejaVu Sans"が実在するため、
        // 「候補が無いから解決できなかった」のではなく「意図的に解決しない」
        // ことを確認するテストになっている。
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let author = parse_stylesheet("p { font-family: sans-serif; }");
        let dom = html::parse(b"<p>text</p>");
        let styles = compute_styles(&dom, &Stylesheet::default(), &author);

        let mut fonts = FontCollection::new(vec![]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert!(
            fonts.is_empty(),
            "sans-serif should not trigger a system font lookup"
        );
    }

    #[test]
    fn load_generic_resolves_monospace_through_the_candidate_list() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let font = system
            .load_generic("monospace", FontWeight::Normal, FontStyle::Normal)
            .expect("DejaVu Sans Mono is in the candidate list and exists in the fixtures");
        assert_eq!(font.family_name().as_deref(), Some("DejaVu Sans Mono"));
    }

    #[test]
    fn load_generic_is_case_insensitive() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        assert!(system
            .load_generic("MONOSPACE", FontWeight::Normal, FontStyle::Normal)
            .is_some());
    }

    #[test]
    fn load_generic_returns_none_for_families_we_deliberately_skip() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        for generic in ["sans-serif", "cursive", "fantasy", "Helvetica"] {
            assert!(
                system
                    .load_generic(generic, FontWeight::Normal, FontStyle::Normal)
                    .is_none(),
                "{generic} should not be resolved as a generic family"
            );
        }
    }

    #[test]
    fn load_generic_returns_none_when_no_candidate_exists() {
        // serifの候補("DejaVu Serif"等)はfixtureに存在しない。monospaceと
        // 違い、フラグによるフォールバック探索も行わないのでNoneになる。
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        assert!(system
            .load_generic("serif", FontWeight::Normal, FontStyle::Normal)
            .is_none());
    }

    #[test]
    fn load_any_monospaced_finds_a_monospaced_face_by_its_metadata_flag() {
        // 候補リストが全て外れた場合のフォールバック経路(fontdbの
        // `FaceInfo::monospaced`フラグ)。fixtureのDejaVu Sans Monoが
        // 等幅フラグを持つことを利用して、経路そのものを直接検証する。
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let font = system
            .load_any_monospaced(FontWeight::Normal, FontStyle::Normal)
            .expect("the fixture directory contains a monospaced face");
        assert_eq!(font.family_name().as_deref(), Some("DejaVu Sans Mono"));
    }

    #[test]
    fn load_missing_system_fonts_registers_monospace_under_the_generic_name() {
        let system = SystemFonts::from_dir(std::path::Path::new(FONTS_DIR));
        let author = parse_stylesheet("pre { font-family: monospace; }");
        let dom = html::parse(b"<pre>text</pre>");
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &author);

        let mut fonts = FontCollection::new(vec![]);
        load_missing_system_fonts(&mut fonts, &styles, &system);

        assert_eq!(fonts.len(), 1);
        assert!(
            fonts.has_family("monospace"),
            "the resolved face must be registered under the generic name so that \
             `font-family: monospace` matches it during selection"
        );
    }
}
