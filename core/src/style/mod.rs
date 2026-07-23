//! CSSパース・セレクタマッチング・カスケード(cssparser/selectors)。

mod cascade;
mod computed;
mod custom_properties;
mod element_ref;
mod extract;
mod font_face;
mod import;
mod page_rule;
mod properties;
mod selector_impl;
mod stylesheet;
mod ua;
mod values;

pub use cascade::matching_declarations;
pub use computed::{
    compute_single_element_style, compute_styles, compute_styles_with_parent,
    resolve_margin_box_content, ComputedBoxShadow, ComputedStyle, FirstLetterStyle, LineHeight,
    RgbaColor,
};
pub use element_ref::ElementRef;
pub use extract::extract_author_stylesheet;
pub use font_face::{FontFaceRule, FontFaceSource};
pub use page_rule::{
    resolve_page_rules, rules_use_page_count, MarginBoxArea, NamedPageSize, PageOrientation,
    PageRule, PageSelector, PageSizeValue, ResolvedPageRule,
};
pub use properties::PropertyDeclaration;
pub use selector_impl::SgSelectorImpl;
pub use stylesheet::{parse_stylesheet, StyleRule, Stylesheet};
pub use ua::user_agent_stylesheet;
pub use values::{
    compose_transform, AlignContent, AlignItems, AlignSelf, BackgroundAttachment,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderCollapse, BorderStyle, BoxSizing,
    BreakBetween, BreakInside, CaptionSide, Clear, Color, ContentPart, CornerRadius, Display,
    EmptyCells, FlexBasis, FlexDirection, FlexWrap, Float, FontStyle, FontWeight, JustifyContent,
    Length, LengthPercentage, LengthPercentageOrAuto, ListStylePosition, ListStyleType, ObjectFit,
    Overflow, Position, QuotePair, TableLayout, TextAlign, TextTransform, TransformFunction,
    VerticalAlign, Visibility, WhiteSpace, ZIndex,
};
