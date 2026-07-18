//! レイアウト結果のPDFオブジェクトへのエンコード。

mod document;
mod font;

pub use document::{encode_pdf, write_document};
pub use font::{embed_font, FontIds};
