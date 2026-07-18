//! ブロック/インラインレイアウトとページ化(taffy + 自前実装)。

mod block;
mod box_tree;
mod geometry;

pub use block::{layout_document, LaidOutBox, LaidOutContent};
pub use box_tree::{build_box_tree, BoxContent, LayoutBox};
pub use geometry::{EdgeSizes, Layout, Rect};
