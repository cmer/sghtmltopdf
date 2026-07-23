//! HTMLパースとDOM構築(html5ever)。

mod dom;
mod parse;

pub use dom::{find_base_href, is_stylesheet_link, Children, Dom, Node, NodeData, NodeId};
pub use parse::{parse, StreamingParser};
