//! `selectors`クレート向けの`SelectorImpl`実装。
//!
//! `selectors`は各種の型([`cssparser::ToCss`]・[`PrecomputedHash`]など)への
//! 実装を要求するが、`html5ever`のアトム型(`LocalName`/`Namespace`)には
//! それらが備わっていないため、`scraper`クレートの実装を参考に薄いラッパー型で包む。

use std::fmt;

use cssparser::ToCss;
use html5ever::{LocalName, Namespace};
use precomputed_hash::PrecomputedHash;
use selectors::parser::{self, SelectorImpl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SgSelectorImpl;

impl SelectorImpl for SgSelectorImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = CssString;
    type Identifier = CssLocalName;
    type LocalName = CssLocalName;
    type NamespacePrefix = CssLocalName;
    type NamespaceUrl = Namespace;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = CssLocalName;
    type NonTSPseudoClass = NonTSPseudoClass;
    type PseudoElement = PseudoElement;
}

/// セレクタパーサ本体。M1では`:is`/`:where`等の拡張構文は扱わない。
#[derive(Clone, Copy, Debug)]
pub struct SelectorParser;

impl<'i> parser::Parser<'i> for SelectorParser {
    type Impl = SgSelectorImpl;
    type Error = parser::SelectorParseErrorKind<'i>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssString(pub String);

impl<'a> From<&'a str> for CssString {
    fn from(value: &'a str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for CssString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for CssString {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_string(&self.0, dest)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct CssLocalName(pub LocalName);

impl<'a> From<&'a str> for CssLocalName {
    fn from(value: &'a str) -> Self {
        Self(value.into())
    }
}

impl PrecomputedHash for CssLocalName {
    fn precomputed_hash(&self) -> u32 {
        self.0.precomputed_hash()
    }
}

impl ToCss for CssLocalName {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(&self.0)
    }
}

/// 非構造的擬似クラス(`:hover`等)。M1では未対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonTSPseudoClass {}

impl parser::NonTSPseudoClass for NonTSPseudoClass {
    type Impl = SgSelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        false
    }

    fn is_user_action_state(&self) -> bool {
        false
    }
}

impl ToCss for NonTSPseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str("")
    }
}

/// 擬似要素(`::before`等)。M1では未対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {}

impl parser::PseudoElement for PseudoElement {
    type Impl = SgSelectorImpl;
}

impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str("")
    }
}
