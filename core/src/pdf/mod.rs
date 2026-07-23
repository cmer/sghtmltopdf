//! レイアウト結果のPDFオブジェクトへのエンコード。

mod document;
mod font;
mod img;
mod streaming;

pub use document::{
    anchor_destination_name, encode_pdf, encode_pdf_with_anchors, write_document, LinkSettings,
};
pub use font::{embed_font, FontIds};
pub use img::{ImageAssetCache, ImagePlane, PlaneColorSpace, PreparedImage};
pub use streaming::StreamingPdfWriter;
