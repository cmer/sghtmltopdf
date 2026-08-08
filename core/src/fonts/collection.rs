//! 複数フォントのコレクションと、`font-family`/weight/style/グリフカバレッジに
//! 基づくフォールバック選択。
//!
//! システムフォント探索(OSのフォントディレクトリを走査すること)は[`super::system`]が担う。

use cssparser::UnicodeRange;

use crate::style::{FontStyle, FontWeight};

use super::font::Font;

/// 数値`font-weight`をBold/Normalの2値に丸める閾値(`style::properties`の
/// `parse_font_weight`と同じ600)。フォント自身のOS/2ウェイト値をこの2値に
/// 丸めて`ComputedStyle::font_weight`と比較できるようにする。
const BOLD_WEIGHT_THRESHOLD: u16 = 600;

pub struct FontCollection {
    fonts: Vec<Font>,
    /// `@font-face`/システムフォントから読み込んだフォントの、CSS上の宣言済み
    /// family名。`None`の要素(`--font`等で明示指定されたフォント)はフォント
    /// 自身の`name`テーブル(`Font::family_name`)で照合する。
    declared_families: Vec<Option<String>>,
    /// `@font-face`のweight/styleディスクリプタによる上書き。`None`の要素は
    /// フォント自身の`OS/2`/`post`テーブルの実メトリクス(`Font::weight`/
    /// `Font::is_italic`)で判定する。
    declared_weights: Vec<Option<FontWeight>>,
    declared_styles: Vec<Option<FontStyle>>,
    /// `@font-face`の`unicode-range`ディスクリプタ。空`Vec`の要素
    /// (`--font`/システムフォント、または`unicode-range`未指定の
    /// `@font-face`)は全域(U+0-10FFFF)を暗黙にカバーするものとして扱う
    declared_unicode_ranges: Vec<Vec<UnicodeRange>>,
    /// 文字カバレッジから自動で見つけたフォールバックとして追加された要素か。
    /// 利用者が明示したフォント(`--font`/`@font-face`)やfamily名から解決した
    /// フォントは`false`。[`Self::can_render_with_matching_face`]が、
    /// 「たまたま入っただけの面」と「利用者が選んだ面」を区別するために使う。
    auto_fallbacks: Vec<bool>,
}

impl FontCollection {
    pub fn new(fonts: Vec<Font>) -> Self {
        let len = fonts.len();
        Self {
            fonts,
            declared_families: vec![None; len],
            declared_weights: vec![None; len],
            declared_styles: vec![None; len],
            declared_unicode_ranges: vec![Vec::new(); len],
            auto_fallbacks: vec![false; len],
        }
    }

    /// `@font-face { font-family: ...; src: url(...); }`やシステムフォントから
    /// 読み込んだフォントを追加する。`family`はフォント自身の`name`テーブルより
    /// 優先してマッチングに使う(フォントファイルの内部名とCSS上の宣言名が
    /// 異なりうるため)。`weight`/`style`は`@font-face`のディスクリプタ値
    /// (CSS側の申告)を渡す。システムフォントのようにCSS側の申告が無い場合は
    /// `None`を渡し、フォント自身の実メトリクスで判定させる。`unicode_range`は
    /// `@font-face`の`unicode-range`ディスクリプタ値で、空`Vec`は
    /// 「未指定(全域カバー)」を意味する。
    pub fn push_font_face(
        &mut self,
        family: String,
        weight: Option<FontWeight>,
        style: Option<FontStyle>,
        unicode_range: Vec<UnicodeRange>,
        font: Font,
    ) {
        self.fonts.push(font);
        self.declared_families.push(Some(family));
        self.declared_weights.push(weight);
        self.declared_styles.push(style);
        self.declared_unicode_ranges.push(unicode_range);
        self.auto_fallbacks.push(false);
    }

    /// 文字カバレッジから自動で見つけたフォールバックフォントを追加する
    /// (`super::system::load_fonts_for_uncovered_chars`用)。
    pub fn push_fallback_font_face(&mut self, family: String, font: Font) {
        self.push_font_face(family, None, None, Vec::new(), font);
        if let Some(flag) = self.auto_fallbacks.last_mut() {
            *flag = true;
        }
    }

    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }

    pub fn get(&self, index: usize) -> Option<&Font> {
        self.fonts.get(index)
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// `families`(CSSの`font-family`リスト、優先順)の名前に一致し、かつ`c`の
    /// グリフを持つフォントのインデックスを返す。
    ///
    /// 選定順序: (1) familyが一致し`c`を描画できるフォントのうち、実際に
    /// `weight`/`style`も満たすもの、(2) familyが一致し`c`を描画できる
    /// フォント(weight/styleは問わない)、(3) familyを問わず`c`を描画できる
    /// 最初のフォント、(4) それでも見つからなければ先頭のフォント
    /// (tofu表示になる)。コレクションが空の場合のみ`None`。
    ///
    /// 選ばれたフォントが実際に要求`weight`/`style`を満たしているかは
    /// [`Self::is_bold`]/[`Self::is_italic`]で別途確認できる(呼び出し側は
    /// これを見て疑似太字/疑似イタリックの要否を判断する)。
    pub fn select_for_char(
        &self,
        families: &[String],
        weight: FontWeight,
        style: FontStyle,
        c: char,
    ) -> Option<usize> {
        if self.fonts.is_empty() {
            return None;
        }

        for family in families {
            if let Some(index) =
                self.best_match(weight, style, c, |i, f| self.matches_family(i, f, family))
            {
                return Some(index);
            }
        }

        // familyがどれも一致しない場合でも、fontを問わずweight/styleが一致する
        // フォントを優先する(既定の"sans-serif"のように、どのフォントの内部
        // family名とも一致しない指定が珍しくないため、ここでも太字/イタリックの
        // 実体選択の機会を諦めない)。
        if let Some(index) = self.best_match(weight, style, c, |_, _| true) {
            return Some(index);
        }

        Some(0)
    }

    /// `eligible`を満たし、`unicode-range`(宣言されていれば)が`c`を含み、
    /// かつ`c`のグリフを持つフォントの中から、`weight`/`style`も実際に
    /// 満たすものを優先して選ぶ(`Self::is_bold`/`Self::is_italic`で判定)。
    /// 一致するものが無ければ、条件を満たす最初のフォントを返す。
    ///
    /// `unicode-range`はハードフィルタとして働く: 宣言されたrangeに`c`が
    /// 含まれない場合、そのフォントが実際に`c`のグリフを持っていても候補
    /// から除外する。走査順(=登録順=CSSソース順)で最初に
    /// 条件を満たしたフォントを採用するため、同じfamily/weight/styleで
    /// rangeが重複する場合は自然に「宣言順(先勝ち)」になる。
    fn best_match(
        &self,
        weight: FontWeight,
        style: FontStyle,
        c: char,
        mut eligible: impl FnMut(usize, &Font) -> bool,
    ) -> Option<usize> {
        let mut first_match = None;
        for (i, f) in self.fonts.iter().enumerate() {
            if !eligible(i, f) || !self.in_unicode_range(i, c) || !f.has_glyph(c) {
                continue;
            }
            first_match.get_or_insert(i);
            if self.is_bold(i) == (weight == FontWeight::Bold)
                && self.is_italic(i) == (style == FontStyle::Italic)
            {
                return Some(i);
            }
        }
        first_match
    }

    /// `index`のフォントの宣言済み`unicode-range`(あれば)に`c`が含まれるか。
    /// `unicode-range`が未宣言(空`Vec`)の場合は常に`true`(全域カバー)。
    fn in_unicode_range(&self, index: usize, c: char) -> bool {
        match self.declared_unicode_ranges.get(index) {
            Some(ranges) if !ranges.is_empty() => {
                let code_point = c as u32;
                ranges
                    .iter()
                    .any(|range| range.start <= code_point && code_point <= range.end)
            }
            _ => true,
        }
    }

    /// `family`に一致するフォント(`--font`/`@font-face`/システムフォント問わず)が
    /// 既にコレクションに含まれているか(weight/styleは問わない)。
    pub fn has_family(&self, family: &str) -> bool {
        self.fonts
            .iter()
            .enumerate()
            .any(|(i, f)| self.matches_family(i, f, family))
    }

    /// `family`に一致し、かつ実際に`weight`/`style`も満たすフォントが
    /// 既にコレクションに含まれているか。システムフォント探索が、既存の
    /// フォントで賄えないweight/style(例: Regularしか無い family宛のBold要求)
    /// だけを補って探すために使う。
    pub fn has_matching_face(&self, family: &str, weight: FontWeight, style: FontStyle) -> bool {
        self.fonts.iter().enumerate().any(|(i, f)| {
            self.matches_family(i, f, family)
                && self.is_bold(i) == (weight == FontWeight::Bold)
                && self.is_italic(i) == (style == FontStyle::Italic)
        })
    }

    /// `c`を実際に描画できるフォントがコレクションにあるか。
    ///
    /// [`Self::select_for_char`]はコレクションが空でない限り必ず何らかの
    /// フォントを返す(最後の手段が「先頭のフォント」=豆腐)ので、豆腐に
    /// なるかどうかを知りたい呼び出し側は、返ってきたフォントが本当に`c`の
    /// グリフを持つかまで確認する必要がある。その判定をまとめたもの。
    pub fn can_render(
        &self,
        families: &[String],
        weight: FontWeight,
        style: FontStyle,
        c: char,
    ) -> bool {
        self.select_for_char(families, weight, style, c)
            .and_then(|index| self.get(index))
            .is_some_and(|font| font.has_glyph(c))
    }

    /// `c`を、要求どおりの`weight`/`style`の面で描画できるか。
    ///
    /// [`Self::can_render`]は「豆腐にならないか」だけを見るので、自動
    /// フォールバックで入ったBold面しか`c`を持たない場合に`true`を返す。
    /// それだけを頼りにフォント探索を打ち切ると、通常ウェイトの文字まで
    /// Bold面で描かれてしまう(文書中に太字の日本語が先に現れると以降の
    /// 日本語が全て太くなる、という出現順依存のバグになる)。
    ///
    /// 一致を要求するのは自動フォールバックの面だけで、利用者が明示した
    /// フォント(`--font`/`@font-face`)しか`c`を持たない場合は`true`のまま
    /// にする。そちらは意図して選ばれた1本なので、勝手にシステムフォントへ
    /// 差し替えず疑似太字/疑似イタリックで賄う。
    pub fn can_render_with_matching_face(
        &self,
        families: &[String],
        weight: FontWeight,
        style: FontStyle,
        c: char,
    ) -> bool {
        let Some(index) = self.select_for_char(families, weight, style, c) else {
            return false;
        };
        if !self.get(index).is_some_and(|font| font.has_glyph(c)) {
            return false;
        }
        if !self.auto_fallbacks.get(index).copied().unwrap_or(false) {
            return true;
        }
        self.is_bold(index) == (weight == FontWeight::Bold)
            && self.is_italic(index) == (style == FontStyle::Italic)
    }

    /// `index`のフォントが実際にBold相当かどうか。`@font-face`の`font-weight`
    /// 申告があればそれを優先し、無ければフォント自身のOS/2ウェイト値で判定する。
    pub fn is_bold(&self, index: usize) -> bool {
        match self.declared_weights.get(index).copied().flatten() {
            Some(weight) => weight == FontWeight::Bold,
            None => self
                .fonts
                .get(index)
                .is_some_and(|f| f.weight() >= BOLD_WEIGHT_THRESHOLD),
        }
    }

    /// `index`のフォントが実際にItalic相当かどうか。`@font-face`の`font-style`
    /// 申告があればそれを優先し、無ければフォント自身の`post`/OS2テーブルの
    /// イタリックフラグで判定する。
    pub fn is_italic(&self, index: usize) -> bool {
        match self.declared_styles.get(index).copied().flatten() {
            Some(style) => style == FontStyle::Italic,
            None => self.fonts.get(index).is_some_and(|f| f.is_italic()),
        }
    }

    fn matches_family(&self, index: usize, font: &Font, family: &str) -> bool {
        match &self.declared_families[index] {
            Some(declared) => declared.eq_ignore_ascii_case(family),
            None => font
                .family_name()
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(family)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
    const DEJAVU_BOLD_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/DejaVuSans-Bold.ttf"
    );
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn dejavu() -> Font {
        Font::load(DEJAVU_PATH).expect("should load bundled DejaVu test font")
    }

    fn dejavu_bold() -> Font {
        Font::load(DEJAVU_BOLD_PATH).expect("should load bundled DejaVu Bold test font")
    }

    fn cjk() -> Font {
        // face index 0 = Noto Sans CJK JP
        Font::load_indexed(CJK_PATH, 0).expect("should load bundled CJK test font")
    }

    fn select(
        collection: &FontCollection,
        family: &str,
        weight: FontWeight,
        style: FontStyle,
        c: char,
    ) -> Option<usize> {
        collection.select_for_char(&[family.to_string()], weight, style, c)
    }

    #[test]
    fn selects_font_matching_family_name_when_it_has_the_glyph() {
        let collection = FontCollection::new(vec![dejavu(), cjk()]);
        let index = select(
            &collection,
            "DejaVu Sans",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn falls_back_to_any_font_that_has_the_glyph_when_family_does_not_match() {
        let collection = FontCollection::new(vec![dejavu(), cjk()]);
        // "sans-serif"はどちらのフォント名にも一致しないので、
        // カバレッジだけで選ばれるはず。
        let index = select(
            &collection,
            "sans-serif",
            FontWeight::Normal,
            FontStyle::Normal,
            '日',
        )
        .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn has_family_reflects_both_own_name_and_declared_overrides() {
        let mut collection = FontCollection::new(vec![dejavu()]);
        assert!(collection.has_family("DejaVu Sans"));
        assert!(!collection.has_family("Custom Brand"));

        collection.push_font_face("Custom Brand".to_string(), None, None, Vec::new(), cjk());
        assert!(collection.has_family("Custom Brand"));
    }

    #[test]
    fn has_matching_face_is_weight_aware_unlike_has_family() {
        let collection = FontCollection::new(vec![dejavu()]);
        // "DejaVu Sans"自体は登録されているが、Regularのみ。Bold要求には
        // 一致しないはず(has_familyはweightを問わないので真になるのと対照的)。
        assert!(collection.has_family("DejaVu Sans"));
        assert!(collection.has_matching_face("DejaVu Sans", FontWeight::Normal, FontStyle::Normal));
        assert!(!collection.has_matching_face("DejaVu Sans", FontWeight::Bold, FontStyle::Normal));
    }

    #[test]
    fn can_render_with_matching_face_rejects_a_fallback_face_of_the_wrong_weight() {
        // 自動フォールバックでBold面だけが入った状態(文書中に太字の日本語が
        // 先に現れたときに起きる)。can_renderは「豆腐にならない」ので真だが、
        // 通常ウェイトの文字をBold面で描くことになるため、探索を続けさせたい。
        let mut collection = FontCollection::new(vec![]);
        collection.push_fallback_font_face("DejaVu Sans".to_string(), dejavu_bold());

        assert!(collection.can_render(&[], FontWeight::Normal, FontStyle::Normal, 'A'));
        assert!(!collection.can_render_with_matching_face(
            &[],
            FontWeight::Normal,
            FontStyle::Normal,
            'A'
        ));
        assert!(collection.can_render_with_matching_face(
            &[],
            FontWeight::Bold,
            FontStyle::Normal,
            'A'
        ));

        // Regular面が足された後は、どちらのウェイトも一致する面で賄える。
        collection.push_fallback_font_face("DejaVu Sans".to_string(), dejavu());
        assert!(collection.can_render_with_matching_face(
            &[],
            FontWeight::Normal,
            FontStyle::Normal,
            'A'
        ));
    }

    #[test]
    fn can_render_with_matching_face_accepts_an_explicit_font_of_the_wrong_weight() {
        // 利用者が明示した1本(`--font`/`@font-face`)しか無い場合は、
        // 勝手にシステムフォントへ差し替えず疑似太字で賄う方針なので、
        // ウェイトが違っても探索を続けさせない。
        let collection = FontCollection::new(vec![dejavu()]);
        assert!(collection.can_render_with_matching_face(
            &[],
            FontWeight::Bold,
            FontStyle::Normal,
            'A'
        ));
    }

    #[test]
    fn font_face_declared_family_takes_priority_over_the_fonts_own_name_table() {
        // 同じDejaVu Sansを2つ登録する: index 0はプレーン(内部name "DejaVu Sans"で照合)、
        // index 1は`@font-face { font-family: "Custom Brand"; }`として読み込んだ体で登録する。
        // "Custom Brand"はどちらのフォントの内部nameとも一致しないので、宣言名の
        // 上書きが効いていなければ名前一致では見つからず、カバレッジのみの
        // フォールバック(先頭=index 0)に落ちてしまい、期待するindex 1にならない。
        let mut collection = FontCollection::new(vec![dejavu()]);
        collection.push_font_face("Custom Brand".to_string(), None, None, Vec::new(), dejavu());

        let index = select(
            &collection,
            "Custom Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn falls_back_to_first_font_when_no_font_has_the_glyph() {
        let collection = FontCollection::new(vec![dejavu()]);
        // DejaVu SansはCJKを含まないので、フォールバックしても先頭(0)になる。
        let index = select(
            &collection,
            "sans-serif",
            FontWeight::Normal,
            FontStyle::Normal,
            '日',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn empty_collection_returns_none() {
        let collection = FontCollection::new(vec![]);
        assert_eq!(
            collection.select_for_char(&[], FontWeight::Normal, FontStyle::Normal, 'A'),
            None
        );
    }

    #[test]
    fn is_bold_reads_the_fonts_own_os2_weight_when_no_font_face_override_is_set() {
        let collection = FontCollection::new(vec![dejavu(), dejavu_bold()]);
        assert!(!collection.is_bold(0));
        assert!(collection.is_bold(1));
    }

    #[test]
    fn is_bold_and_is_italic_prefer_the_font_face_declared_override() {
        let mut collection = FontCollection::new(vec![]);
        // 実体はRegularのDejaVu Sansだが、`@font-face { font-weight: bold; font-style: italic; }`
        // として読み込まれた体で登録する。実メトリクスではなく申告値が優先されるはず。
        collection.push_font_face(
            "Declared Brand".to_string(),
            Some(FontWeight::Bold),
            Some(FontStyle::Italic),
            Vec::new(),
            dejavu(),
        );
        assert!(collection.is_bold(0));
        assert!(collection.is_italic(0));
    }

    #[test]
    fn select_for_char_prefers_the_real_bold_face_over_the_regular_one() {
        let collection = FontCollection::new(vec![dejavu(), dejavu_bold()]);
        // どちらもfamily名"DejaVu Sans"で一致するが、weight: Boldを要求したら
        // 実際にBoldなindex 1が選ばれるはず(index 0は疑似太字に頼らずに済む)。
        let index = select(
            &collection,
            "DejaVu Sans",
            FontWeight::Bold,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn select_for_char_falls_back_to_the_regular_face_when_no_bold_face_matches() {
        let collection = FontCollection::new(vec![dejavu()]);
        // Boldなフォントが無い場合は、家族名一致するRegularフォントにフォール
        // バックする(呼び出し側が疑似太字で補う)。
        let index = select(
            &collection,
            "DejaVu Sans",
            FontWeight::Bold,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 0);
        assert!(!collection.is_bold(index));
    }

    #[test]
    fn unicode_range_excludes_a_font_even_when_it_has_the_glyph() {
        // index 0はグリフとしては'é'(U+00E9)を実際に持つDejaVu Sansだが、
        // `unicode-range: U+0-7F`(Basic Latinのみ)を宣言しているため対象外。
        // index 1は同じDejaVu Sansをrange指定なしで再登録したもの。
        // ハードフィルタが効いていれば、グリフの有無に関わらずindex 0は
        // 除外されindex 1が選ばれるはず。
        let mut collection = FontCollection::new(vec![]);
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x7F,
            }],
            dejavu(),
        );
        collection.push_font_face("Brand".to_string(), None, None, Vec::new(), dejavu());

        assert!(
            collection.get(0).unwrap().has_glyph('é'),
            "テスト前提: DejaVu Sansは'é'のグリフを持つはず"
        );

        let index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            'é',
        )
        .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn unicode_range_does_not_exclude_a_char_inside_the_declared_range() {
        let mut collection = FontCollection::new(vec![]);
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x7F,
            }],
            dejavu(),
        );

        let index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn unicode_range_unspecified_covers_the_whole_unicode_range() {
        // 既存の挙動の後方互換確認: unicode_rangeを指定しない登録は
        // これまで通り全域をカバーする。
        let collection = FontCollection::new(vec![dejavu()]);
        let index = select(
            &collection,
            "DejaVu Sans",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn overlapping_unicode_ranges_prefer_the_first_declared_font() {
        // 同じfamily・weight/style・重複するrangeを持つ2つのフォントが
        // 同じ文字をカバーする場合、CSSソース中で先に登録された方
        // (登録順=走査順)が優先されるはず。
        let mut collection = FontCollection::new(vec![]);
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x7F,
            }],
            dejavu(),
        );
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0xFF,
            }],
            dejavu_bold(),
        );

        let index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn falls_back_to_tofu_when_every_font_excludes_the_char_by_range() {
        let mut collection = FontCollection::new(vec![]);
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x7F,
            }],
            dejavu(),
        );

        let index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            '日',
        )
        .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn unicode_range_splits_a_latin_and_a_cjk_face_declared_under_the_same_family() {
        // 典型的なwebfont配信パターン: 英数字用フォントとCJK用フォントを
        // 同じfamily名でunicode-range分けして併用する。
        let mut collection = FontCollection::new(vec![]);
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x0,
                end: 0x24F,
            }],
            dejavu(),
        );
        collection.push_font_face(
            "Brand".to_string(),
            None,
            None,
            vec![UnicodeRange {
                start: 0x4E00,
                end: 0x9FFF,
            }],
            cjk(),
        );

        let latin_index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            'A',
        )
        .unwrap();
        assert_eq!(
            latin_index, 0,
            "Latin characters should use the Latin-range face"
        );

        let cjk_index = select(
            &collection,
            "Brand",
            FontWeight::Normal,
            FontStyle::Normal,
            '日',
        )
        .unwrap();
        assert_eq!(cjk_index, 1, "CJK characters should use the CJK-range face");
    }
}
