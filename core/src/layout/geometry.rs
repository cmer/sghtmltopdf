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

/// ページ分割によって、元のボックスが複数ページにまたがる断片
/// (フラグメント)に分けられているかどうか。
///
/// `border-radius`は要素の計算スタイル(`border_top_left_radius`等)から
/// 都度引くため、ページをまたいで分割されたボックスの「継続中」の断片
/// (先頭でも末尾でもない`Middle`、あるいは先頭のみの`First`の下端・
/// 末尾のみの`Last`の上端)では、本来枠線が無い辺の角に丸みを適用しては
/// ならない。この情報がないと[`crate::pdf::document`]の描画側で
/// 区別がつかないため、[`Layout`]に持たせる。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FragmentPosition {
    /// 分割されていない(通常の)ボックス。全角に`border-radius`を適用してよい。
    #[default]
    Whole,
    /// 分割された断片のうち最初のもの。上端の角のみ`border-radius`を適用してよい。
    First,
    /// 分割された断片のうち最初でも最後でもないもの。どの角にも適用しない。
    Middle,
    /// 分割された断片のうち最後のもの。下端の角のみ`border-radius`を適用してよい。
    Last,
}

/// ボックスモデルの各領域。`content`のみ絶対座標(ページ内)を持ち、
/// 他の辺はその太さのみを保持する。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Layout {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
    pub fragment: FragmentPosition,
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

    /// `overflow`のクリップ境界に使うパディングボックス(content+padding、
    /// border線の内側、[0023](../../../docs/decisions/0023-box-model-details-design.md)
    /// 決定1)。
    pub fn padding_box(&self) -> Rect {
        Rect {
            x: self.content.x - self.padding.left,
            y: self.content.y - self.padding.top,
            width: self.padding.left + self.content.width + self.padding.right,
            height: self.padding.top + self.content.height + self.padding.bottom,
        }
    }
}
