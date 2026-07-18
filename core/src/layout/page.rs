//! ページサイズ・マージンの定義。ページの内側(コンテンツ領域)が
//! containing blockとなる。
//!
//! 単位は他のレイアウト計算と同様CSS px(96dpi基準)。PDFのポイント単位への
//! 変換はPDF出力(T9)の責務とする。

use super::geometry::EdgeSizes;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    /// 210mm × 297mm(96dpi換算)。
    pub const A4: PageSize = PageSize {
        width: 793.7,
        height: 1122.5,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSettings {
    pub size: PageSize,
    pub margin: EdgeSizes,
}

impl Default for PageSettings {
    /// 1インチ(96px)相当の既定マージン。
    fn default() -> Self {
        Self {
            size: PageSize::A4,
            margin: EdgeSizes {
                top: 96.0,
                right: 96.0,
                bottom: 96.0,
                left: 96.0,
            },
        }
    }
}

impl PageSettings {
    pub fn content_width(&self) -> f32 {
        self.size.width - self.margin.left - self.margin.right
    }

    pub fn content_height(&self) -> f32 {
        self.size.height - self.margin.top - self.margin.bottom
    }
}
