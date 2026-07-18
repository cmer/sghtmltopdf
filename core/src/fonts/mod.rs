//! フォント読み込みとシェイピング(rustybuzz/ttf-parser)。

mod font;
mod shape;

pub use font::{Font, FontLoadError};
pub use shape::{measure_text, shape_text, ShapedGlyph, ShapedText};
