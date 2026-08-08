//! ページサイズ・マージンの定義。ページの内側(コンテンツ領域)がcontaining blockとなる。

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
    /// 297mm × 420mm(96dpi換算)。
    pub const A3: PageSize = PageSize {
        width: 1122.5,
        height: 1587.4,
    };
    /// 148mm × 210mm(96dpi換算)。
    pub const A5: PageSize = PageSize {
        width: 559.4,
        height: 793.7,
    };
    /// 8.5in × 11in(96dpi換算、`@page`の`size: letter`用)。
    pub const LETTER: PageSize = PageSize {
        width: 816.0,
        height: 1056.0,
    };
    /// 8.5in × 14in(96dpi換算)。
    pub const LEGAL: PageSize = PageSize {
        width: 816.0,
        height: 1344.0,
    };

    /// 幅・高さを入れ替える(`landscape`修飾子用)。
    pub fn landscape(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
    }
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
