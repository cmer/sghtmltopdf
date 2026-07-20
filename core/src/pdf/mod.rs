//! レイアウト結果のPDFオブジェクトへのエンコード。

mod document;
mod font;
mod img;
mod streaming;

pub use document::{encode_pdf, write_document};
pub use font::{embed_font, FontIds};
pub use img::{ImageAssetCache, ImagePlane, PlaneColorSpace, PreparedImage};
pub use streaming::StreamingPdfWriter;
