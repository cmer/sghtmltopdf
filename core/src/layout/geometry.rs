//! レイアウト結果の座標・矩形の型。

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// ボックスモデルの各領域。`content`のみ絶対座標(ページ内)を持ち、
/// 他の辺はその太さのみを保持する。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Layout {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Layout {
    /// 次の兄弟ボックスまでの垂直方向の占有量(マージンボックスの高さ)。
    pub fn margin_box_height(&self) -> f32 {
        self.margin.top
            + self.border.top
            + self.padding.top
            + self.content.height
            + self.padding.bottom
            + self.border.bottom
            + self.margin.bottom
    }

    /// 背景・枠線の描画に使うボーダーボックス。
    pub fn border_box(&self) -> Rect {
        Rect {
            x: self.content.x - self.padding.left - self.border.left,
            y: self.content.y - self.padding.top - self.border.top,
            width: self.border.left
                + self.padding.left
                + self.content.width
                + self.padding.right
                + self.border.right,
            height: self.border.top
                + self.padding.top
                + self.content.height
                + self.padding.bottom
                + self.border.bottom,
        }
    }
}
