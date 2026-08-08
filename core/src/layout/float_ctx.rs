//! `float`の配置を追跡する簡易コンテキスト(CSS2.1 9.5.1のshelf-packing簡略版)。
//!
//! `layout_document`/`layout_document_from`1回の呼び出し全体で1つの
//! [`FloatContext`]を共有する(このリポジトリは`float`以外にBlock Formatting
//! Contextを確立するプロパティを実装していないため)。`float`自身の内容・
//! `display: table`のセルの内容には新しい空のコンテキストを渡す
//! (この2つは新BFCを確立するため)。

use crate::style::{Clear, Float};

/// 絶対(ページ内)座標で表した1つのfloatの矩形。`inner_edge_x`は回り込み判定に
/// 必要な内側境界のみ(左floatなら右端、右floatなら左端)を保持する。
#[derive(Debug, Clone, Copy)]
struct FloatEntry {
    top: f32,
    bottom: f32,
    inner_edge_x: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FloatContext {
    left: Vec<FloatEntry>,
    right: Vec<FloatEntry>,
}

impl FloatContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// `side`方向にfloatを配置する絶対座標(margin boxの左上)を求める。
    /// `preferred_top`(=floatがDOMの流れに出現した時点のcursor_y)から探索を
    /// 開始し、そのY地点での同方向floatの占有幅+新規floatの幅が
    /// `containing_right - containing_left`を超えない最初のYを採用する。
    /// 超える場合は、そのY時点で重なっている同方向floatのうち最も浅い最下端まで
    /// Yを進めて再試行する(shelf-packing)。
    pub fn place(
        &self,
        side: Float,
        preferred_top: f32,
        containing_left: f32,
        containing_right: f32,
        margin_box_width: f32,
    ) -> (f32, f32) {
        let entries: &[FloatEntry] = match side {
            Float::Left => &self.left,
            Float::Right => &self.right,
            Float::None => return (containing_left, preferred_top),
        };

        let mut y = preferred_top;
        loop {
            let overlapping: Vec<&FloatEntry> = entries
                .iter()
                .filter(|e| e.top <= y && y < e.bottom)
                .collect();

            let (available_left, available_right) = if side == Float::Left {
                (
                    overlapping
                        .iter()
                        .map(|e| e.inner_edge_x)
                        .fold(containing_left, f32::max),
                    containing_right,
                )
            } else {
                (
                    containing_left,
                    overlapping
                        .iter()
                        .map(|e| e.inner_edge_x)
                        .fold(containing_right, f32::min),
                )
            };

            if available_right - available_left >= margin_box_width {
                let x = if side == Float::Left {
                    available_left
                } else {
                    available_right - margin_box_width
                };
                return (x, y);
            }

            let next_y = overlapping
                .iter()
                .map(|e| e.bottom)
                .fold(f32::INFINITY, f32::min);
            if next_y.is_finite() && next_y > y {
                y = next_y;
                continue;
            }

            // 進める先がない(containing widthよりmargin_box_width自体が大きい等):
            // 無限ループを避け、best-effortでオーバーフローを許容してそのまま確定する。
            let x = if side == Float::Left {
                available_left
            } else {
                available_right - margin_box_width
            };
            return (x, y);
        }
    }

    /// 配置確定後にfloatを登録する。
    pub fn register(
        &mut self,
        side: Float,
        x: f32,
        y: f32,
        margin_box_width: f32,
        margin_box_height: f32,
    ) {
        let entry = FloatEntry {
            top: y,
            bottom: y + margin_box_height,
            inner_edge_x: if side == Float::Left {
                x + margin_box_width
            } else {
                x
            },
        };
        match side {
            Float::Left => self.left.push(entry),
            Float::Right => self.right.push(entry),
            Float::None => {}
        }
    }

    /// `y`〜`y+height`の帯で、floatに占有されていない`(available_left,
    /// available_width)`を返す(`inline.rs`が行ごとの折り返し判定に使う)。
    pub fn available_band(
        &self,
        y: f32,
        height: f32,
        containing_left: f32,
        containing_right: f32,
    ) -> (f32, f32) {
        let overlaps = |e: &&FloatEntry| e.top < y + height && y < e.bottom;

        let left_edge = self
            .left
            .iter()
            .filter(overlaps)
            .map(|e| e.inner_edge_x)
            .fold(containing_left, f32::max);
        let right_edge = self
            .right
            .iter()
            .filter(overlaps)
            .map(|e| e.inner_edge_x)
            .fold(containing_right, f32::min);

        (left_edge, (right_edge - left_edge).max(0.0))
    }

    /// `clear`方向のfloat最下端まで押し下げた後のY(対象floatが無ければ`current_y`)。
    pub fn clearance(&self, clear: Clear, current_y: f32) -> f32 {
        let max_bottom =
            |entries: &[FloatEntry]| entries.iter().map(|e| e.bottom).fold(current_y, f32::max);
        match clear {
            Clear::None => current_y,
            Clear::Left => max_bottom(&self.left),
            Clear::Right => max_bottom(&self.right),
            Clear::Both => max_bottom(&self.left).max(max_bottom(&self.right)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_float_places_at_preferred_top_against_containing_edge() {
        let ctx = FloatContext::new();
        assert_eq!(
            ctx.place(Float::Left, 100.0, 0.0, 500.0, 120.0),
            (0.0, 100.0)
        );
        assert_eq!(
            ctx.place(Float::Right, 100.0, 0.0, 500.0, 120.0),
            (380.0, 100.0)
        );
    }

    #[test]
    fn second_left_float_packs_next_to_the_first() {
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 100.0, 50.0);
        assert_eq!(ctx.place(Float::Left, 0.0, 0.0, 500.0, 100.0), (100.0, 0.0));
    }

    #[test]
    fn third_float_wraps_to_next_shelf_when_it_does_not_fit() {
        let mut ctx = FloatContext::new();
        // 短いが幅の広いfloat(y:0-30でx:100-300を占有)。
        ctx.register(Float::Left, 100.0, 0.0, 200.0, 30.0);
        // 高さはあるが幅の狭いfloat(y:0-200でx:0-50を占有)。
        ctx.register(Float::Left, 0.0, 0.0, 50.0, 200.0);

        // 幅400のfloatはy=0時点では収まらない(占有幅300、空き200)。1番目のfloatが
        // 抜けるy=30まで進めば、残る占有は50のみとなり幅450の空きが生まれ収まる。
        assert_eq!(ctx.place(Float::Left, 0.0, 0.0, 500.0, 400.0), (50.0, 30.0));
    }

    #[test]
    fn place_overflows_when_float_is_wider_than_containing_block() {
        let ctx = FloatContext::new();
        assert_eq!(ctx.place(Float::Left, 0.0, 0.0, 100.0, 200.0), (0.0, 0.0));
    }

    #[test]
    fn available_band_narrows_around_overlapping_floats() {
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 100.0, 50.0);
        ctx.register(Float::Right, 400.0, 0.0, 100.0, 50.0);
        assert_eq!(ctx.available_band(10.0, 20.0, 0.0, 500.0), (100.0, 300.0));
    }

    #[test]
    fn available_band_ignores_floats_outside_the_vertical_band() {
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 100.0, 50.0);
        assert_eq!(ctx.available_band(100.0, 20.0, 0.0, 500.0), (0.0, 500.0));
    }

    #[test]
    fn clearance_pushes_down_to_the_bottom_of_relevant_floats() {
        let mut ctx = FloatContext::new();
        ctx.register(Float::Left, 0.0, 0.0, 100.0, 50.0);
        ctx.register(Float::Right, 400.0, 10.0, 100.0, 80.0);

        assert_eq!(ctx.clearance(Clear::None, 5.0), 5.0);
        assert_eq!(ctx.clearance(Clear::Left, 5.0), 50.0);
        assert_eq!(ctx.clearance(Clear::Right, 5.0), 90.0);
        assert_eq!(ctx.clearance(Clear::Both, 5.0), 90.0);
    }
}
