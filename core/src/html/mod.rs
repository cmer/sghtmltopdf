//! HTMLパースとDOM構築(html5ever)。

mod dom;
mod parse;

mod encoding;

pub use dom::{
    collect_anchor_targets, find_base_href, find_document_title, is_stylesheet_link, Children, Dom,
    Node, NodeData, NodeId,
};
pub use encoding::decode_html;
pub use parse::{parse, StreamingParser};
