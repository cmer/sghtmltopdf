//! フォントファイルの読み込み。
//!
//! M1ではローカルパス指定の最小実装のみ対応する。システムフォント探索や
//! `@font-face`によるwebfont解決は将来のマイルストーンで扱う。

use std::fmt;
use std::path::Path;

/// 読み込み済みのフォントデータ。
///
/// ファイルの生バイト列を所有し、シェイピングに必要な`rustybuzz::Face`は
/// 呼び出しのたびに借用ビューとして構築する(`Face`自体はライフタイムを
/// 持つため、`Font`に保持させると自己参照構造体になってしまうのを避けるため)。
#[derive(Debug, Clone)]
pub struct Font {
    data: Vec<u8>,
    index: u32,
}

#[derive(Debug)]
pub struct FontLoadError(String);

impl fmt::Display for FontLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "フォントの読み込みに失敗しました: {}", self.0)
    }
}

impl std::error::Error for FontLoadError {}

impl Font {
    /// ローカルファイルパスからフォントを読み込む。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FontLoadError> {
        Self::load_indexed(path, 0)
    }

    /// ローカルファイルパスからフォントを読み込む。TrueType Collection(`.ttc`)等、
    /// 複数フェイスを含むファイルの場合は`index`でフェイスを選択する。
    pub fn load_indexed(path: impl AsRef<Path>, index: u32) -> Result<Self, FontLoadError> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).map_err(|e| FontLoadError(format!("{}: {e}", path.display())))?;
        Self::from_bytes(data, index)
    }

    /// 読み込み済みのバイト列からフォントを構築する(TrueType Collection等、
    /// 複数フェイスを含む場合は`index`でフェイスを選択する)。
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, FontLoadError> {
        if rustybuzz::Face::from_slice(&data, index).is_none() {
            return Err(FontLoadError("不正なフォントデータです".to_string()));
        }
        Ok(Self { data, index })
    }

    pub(crate) fn face(&self) -> rustybuzz::Face<'_> {
        rustybuzz::Face::from_slice(&self.data, self.index)
            .expect("Font構築時に検証済みのため、ここでのパース失敗はありえない")
    }

    /// フォントファイルの生バイト列(PDFへのフォント埋め込み等で必要)。
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイル内でのフェイス番号。
    pub fn face_index(&self) -> u32 {
        self.index
    }

    pub fn units_per_em(&self) -> u16 {
        self.face().units_per_em() as u16
    }

    pub fn ascender(&self) -> i16 {
        self.face().ascender()
    }

    pub fn descender(&self) -> i16 {
        self.face().descender()
    }

    pub fn capital_height(&self) -> Option<i16> {
        self.face().capital_height()
    }

    pub fn italic_angle(&self) -> f32 {
        self.face().italic_angle()
    }

    pub fn is_italic(&self) -> bool {
        self.face().is_italic()
    }

    pub fn is_monospaced(&self) -> bool {
        self.face().is_monospaced()
    }

    /// OS/2テーブルのウェイト値(400=標準, 700=太字)。
    pub fn weight(&self) -> u16 {
        self.face().weight().to_number()
    }

    pub fn bounding_box(&self) -> ttf_parser::Rect {
        self.face().global_bounding_box()
    }

    /// `glyph_id`の水平アドバンス幅(フォントユニット)。
    pub fn glyph_hor_advance(&self, glyph_id: u16) -> Option<u16> {
        self.face().glyph_hor_advance(ttf_parser::GlyphId(glyph_id))
    }

    /// `c`に対応するグリフをこのフォントが持っているか。
    /// font-familyフォールバック(どのフォントでこの文字を描画できるか)の判定に使う。
    pub fn has_glyph(&self, c: char) -> bool {
        self.face().glyph_index(c).is_some()
    }

    /// フォント名(`name`テーブルの Typographic Family、無ければ Family)。
    /// Unicodeエンコードの英語名のみ対応する。
    pub fn family_name(&self) -> Option<String> {
        let face = self.face();
        let names = face.names();

        let pick = |id: u16| {
            names
                .into_iter()
                .find(|n| n.name_id == id && n.is_unicode())
                .and_then(|n| n.to_string())
        };

        pick(ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| pick(ttf_parser::name_id::FAMILY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    #[test]
    fn loads_a_valid_font_file() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert!(font.face().units_per_em() > 0);
    }

    #[test]
    fn load_fails_for_missing_file() {
        let result = Font::load("/nonexistent/path/does-not-exist.ttf");
        assert!(result.is_err());
    }

    #[test]
    fn reports_family_name() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert_eq!(font.family_name().as_deref(), Some("DejaVu Sans"));
    }

    #[test]
    fn has_glyph_distinguishes_covered_and_uncovered_characters() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert!(font.has_glyph('A'));
        // DejaVu SansはCJK文字を含まない。
        assert!(!font.has_glyph('日'));
    }

    #[test]
    fn from_bytes_rejects_invalid_font_data() {
        let result = Font::from_bytes(b"not a font file".to_vec(), 0);
        assert!(result.is_err());
    }
}
