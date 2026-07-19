//! 複数フォントのコレクションと、`font-family`/グリフカバレッジに基づく
//! フォールバック選択。
//!
//! システムフォント探索(OSのフォントディレクトリを走査すること)は将来の
//! マイルストーンで扱う。ここでは呼び出し側が明示的に読み込んだフォントの
//! 中から選ぶだけの、いわば「手動フォールバックチェーン」を提供する。

use super::font::Font;

pub struct FontCollection {
    fonts: Vec<Font>,
    /// `@font-face`から読み込んだフォントの、CSS上の宣言済みfamily名。
    /// `None`の要素(`--font`等で明示指定されたフォント)はフォント自身の
    /// `name`テーブル(`Font::family_name`)で照合する。
    declared_families: Vec<Option<String>>,
}

impl FontCollection {
    pub fn new(fonts: Vec<Font>) -> Self {
        let declared_families = vec![None; fonts.len()];
        Self {
            fonts,
            declared_families,
        }
    }

    /// `@font-face { font-family: ...; src: url(...); }`から読み込んだフォントを
    /// 追加する。`family`はフォント自身の`name`テーブルより優先してマッチングに使う
    /// (フォントファイルの内部名とCSS上の宣言名が異なりうるため)。
    pub fn push_font_face(&mut self, family: String, font: Font) {
        self.fonts.push(font);
        self.declared_families.push(Some(family));
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
    /// 選定順序: (1) `families`に名前が一致し`c`を描画できるフォント、
    /// (2) 名前を問わず`c`を描画できる最初のフォント、
    /// (3) それでも見つからなければ先頭のフォント(tofu表示になる)。
    /// コレクションが空の場合のみ`None`。
    ///
    /// 既知の簡略化: `font-weight`/`font-style`は考慮しない(同じfamily名で
    /// Regular/Boldを別々に`@font-face`登録していても、太字が要求される場面で
    /// 自動選択はされない。太字/イタリックは引き続き疑似合成で描画される)。
    pub fn select_for_char(&self, families: &[String], c: char) -> Option<usize> {
        if self.fonts.is_empty() {
            return None;
        }

        for family in families {
            if let Some(index) = self
                .fonts
                .iter()
                .enumerate()
                .position(|(i, f)| self.matches_family(i, f, family))
            {
                if self.fonts[index].has_glyph(c) {
                    return Some(index);
                }
            }
        }

        if let Some(index) = self.fonts.iter().position(|f| f.has_glyph(c)) {
            return Some(index);
        }

        Some(0)
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
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn dejavu() -> Font {
        Font::load(DEJAVU_PATH).expect("should load bundled DejaVu test font")
    }

    fn cjk() -> Font {
        // face index 0 = Noto Sans CJK JP
        Font::load_indexed(CJK_PATH, 0).expect("should load bundled CJK test font")
    }

    #[test]
    fn selects_font_matching_family_name_when_it_has_the_glyph() {
        let collection = FontCollection::new(vec![dejavu(), cjk()]);
        let index = collection
            .select_for_char(&["DejaVu Sans".to_string()], 'A')
            .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn falls_back_to_any_font_that_has_the_glyph_when_family_does_not_match() {
        let collection = FontCollection::new(vec![dejavu(), cjk()]);
        // "sans-serif"はどちらのフォント名にも一致しないので、
        // カバレッジだけで選ばれるはず。
        let index = collection
            .select_for_char(&["sans-serif".to_string()], '日')
            .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn font_face_declared_family_takes_priority_over_the_fonts_own_name_table() {
        // 同じDejaVu Sansを2つ登録する: index 0はプレーン(内部name "DejaVu Sans"で照合)、
        // index 1は`@font-face { font-family: "Custom Brand"; }`として読み込んだ体で登録する。
        // "Custom Brand"はどちらのフォントの内部nameとも一致しないので、宣言名の
        // 上書きが効いていなければ名前一致では見つからず、カバレッジのみの
        // フォールバック(先頭=index 0)に落ちてしまい、期待するindex 1にならない。
        let mut collection = FontCollection::new(vec![dejavu()]);
        collection.push_font_face("Custom Brand".to_string(), dejavu());

        let index = collection
            .select_for_char(&["Custom Brand".to_string()], 'A')
            .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn falls_back_to_first_font_when_no_font_has_the_glyph() {
        let collection = FontCollection::new(vec![dejavu()]);
        // DejaVu SansはCJKを含まないので、フォールバックしても先頭(0)になる。
        let index = collection
            .select_for_char(&["sans-serif".to_string()], '日')
            .unwrap();
        assert_eq!(index, 0);
    }

    #[test]
    fn empty_collection_returns_none() {
        let collection = FontCollection::new(vec![]);
        assert_eq!(collection.select_for_char(&[], 'A'), None);
    }
}
