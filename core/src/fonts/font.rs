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
        let path = path.as_ref();
        let data =
            std::fs::read(path).map_err(|e| FontLoadError(format!("{}: {e}", path.display())))?;
        Self::from_bytes(data, 0)
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
    fn from_bytes_rejects_invalid_font_data() {
        let result = Font::from_bytes(b"not a font file".to_vec(), 0);
        assert!(result.is_err());
    }
}
