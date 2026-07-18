//! ブロック/インラインレイアウトとページ化(taffy + 自前実装)。

mod block;
mod box_tree;
mod geometry;
mod inline;
mod page;
mod paginate;

pub use block::{layout_document, LaidOutBox, LaidOutContent};
pub use box_tree::{build_box_tree, BoxContent, LayoutBox};
pub use geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
pub use inline::{layout_inline_content, LineBox, TextRun};
pub use page::{PageSettings, PageSize};
pub use paginate::{paginate, paginate_document, Page};
