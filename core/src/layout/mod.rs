//! ブロック/インラインレイアウトとページ化(taffy + 自前実装)。

mod block;
mod box_tree;
mod geometry;
mod inline;
mod page;
mod paginate;
mod table;

pub use block::{layout_document, LaidOutBox, LaidOutContent, LaidOutTableRow};
pub use box_tree::{build_box_tree, BoxContent, LayoutBox, TableBox, TableCell, TableRow};
pub use geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
pub use inline::{layout_inline_content, LineBox, TextRun};
pub use page::{PageSettings, PageSize};
pub use paginate::{
    paginate, paginate_document, paginate_document_streaming, paginate_streaming, Page,
};
