//! レイアウト結果のPDFオブジェクトへのエンコード。

mod document;
mod font;
mod streaming;

pub use document::{encode_pdf, write_document};
pub use font::{embed_font, FontIds};
pub use streaming::StreamingPdfWriter;
