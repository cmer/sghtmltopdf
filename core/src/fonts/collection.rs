//! 複数フォントのコレクションと、`font-family`/グリフカバレッジに基づく
//! フォールバック選択。
//!
//! システムフォント探索(OSのフォントディレクトリを走査すること)は将来の
//! マイルストーンで扱う。ここでは呼び出し側が明示的に読み込んだフォントの
//! 中から選ぶだけの、いわば「手動フォールバックチェーン」を提供する。

use super::font::Font;

pub struct FontCollection {
    fonts: Vec<Font>,
}

impl FontCollection {
    pub fn new(fonts: Vec<Font>) -> Self {
        Self { fonts }
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
    pub fn select_for_char(&self, families: &[String], c: char) -> Option<usize> {
        if self.fonts.is_empty() {
            return None;
        }

        for family in families {
            if let Some(index) = self.fonts.iter().position(|f| {
                f.family_name()
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(family))
            }) {
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
