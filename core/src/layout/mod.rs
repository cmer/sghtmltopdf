//! ブロック/インラインレイアウトとページ化(taffy + 自前実装)。

mod block;
mod box_tree;
mod geometry;
mod inline;
mod page;
mod paginate;
mod table;

pub(crate) use block::{
    has_visible_decoration, resolve_border, resolve_lpa_or_zero, resolve_padding,
    resolve_width_and_horizontal_margins,
};
pub use block::{
    layout_document, layout_document_from, LaidOutBox, LaidOutContent, LaidOutTableRow,
};
pub(crate) use box_tree::build_box_for_element;
pub use box_tree::{
    build_box_tree, resolve_images, BoxContent, ImageBoxContent, LayoutBox, TableBox, TableCell,
    TableRow,
};
pub use geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
pub use inline::{layout_inline_content, LineBox, TextRun};
pub use page::{PageSettings, PageSize};
pub(crate) use paginate::collect_completed_subtree_roots;
pub use paginate::{
    paginate, paginate_document, paginate_document_streaming, paginate_streaming, Page,
    StreamingPaginator,
};
