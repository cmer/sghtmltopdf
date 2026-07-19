//! フォント読み込みとシェイピング(rustybuzz/ttf-parser)。

mod collection;
mod face;
mod font;
mod shape;
mod system;

pub use collection::FontCollection;
pub use face::{load_font_faces, LoadedFontFace};
pub use font::{Font, FontLoadError};
pub use shape::{measure_text, shape_text, ShapedGlyph, ShapedText};
pub use system::{load_missing_system_fonts, SystemFonts};
