//! フォント読み込みとシェイピング(rustybuzz/ttf-parser)。

mod collection;
mod face;
mod font;
mod shape;

pub use collection::FontCollection;
pub use face::{load_font_faces, LoadedFontFace};
pub use font::{Font, FontLoadError};
pub use shape::{measure_text, shape_text, ShapedGlyph, ShapedText};
