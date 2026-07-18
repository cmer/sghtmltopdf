//! CSSパース・セレクタマッチング・カスケード(cssparser/selectors)。

mod cascade;
mod element_ref;
mod properties;
mod selector_impl;
mod stylesheet;
mod ua;
mod values;

pub use cascade::matching_declarations;
pub use element_ref::ElementRef;
pub use properties::PropertyDeclaration;
pub use selector_impl::SgSelectorImpl;
pub use stylesheet::{parse_stylesheet, StyleRule, Stylesheet};
pub use ua::user_agent_stylesheet;
pub use values::{Color, Display, Length, LengthPercentage, LengthPercentageOrAuto};
