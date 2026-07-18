//! フォント読み込みとシェイピング(rustybuzz/ttf-parser)。

mod collection;
mod font;
mod shape;

pub use collection::FontCollection;
pub use font::{Font, FontLoadError};
pub use shape::{measure_text, shape_text, ShapedGlyph, ShapedText};
