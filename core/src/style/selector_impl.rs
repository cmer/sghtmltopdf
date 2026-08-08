//! `selectors`クレート向けの`SelectorImpl`実装。
//!
//! `selectors`は各種の型([`cssparser::ToCss`]・[`PrecomputedHash`]など)への
//! 実装を要求するが、`html5ever`のアトム型(`LocalName`/`Namespace`)には
//! それらが備わっていないため、`scraper`クレートの実装を参考に薄いラッパー型で包む。

use std::fmt;

use cssparser::{match_ignore_ascii_case, CowRcStr, SourceLocation, ToCss};
use html5ever::{LocalName, Namespace};
use precomputed_hash::PrecomputedHash;
use selectors::parser::{self, SelectorImpl, SelectorParseErrorKind};

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

/// セレクタパーサ本体。`:is`/`:where`等の拡張構文は扱わない。
#[derive(Clone, Copy, Debug)]
pub struct SelectorParser;

impl<'i> parser::Parser<'i> for SelectorParser {
    type Impl = SgSelectorImpl;
    type Error = parser::SelectorParseErrorKind<'i>;

    /// `:hover`等の非構造的擬似クラスはパース自体には対応する(常に非マッチとして
    /// 扱うのは[`super::element_ref::ElementRef::match_non_ts_pseudo_class`]の役割)。
    ///
    /// これらを未対応のまま(空enumの`NonTSPseudoClass`のまま)にしておくと、
    /// `selectors`クレートの`SelectorList::parse`は非寛容(1つでも無効なセレクタが
    /// あるとリスト全体を`Err`にする)なため、`.foo, .bar:hover { ... }`のように
    /// カンマ区切りの一部だけが`:hover`を含む場合でも、無関係な`.foo`の宣言まで
    /// ルールごと消えてしまう。パースを成功させることでこの巻き添えを防ぐ。
    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<NonTSPseudoClass, cssparser::ParseError<'i, Self::Error>> {
        Ok(match_ignore_ascii_case! { &name,
            "hover" => NonTSPseudoClass::Hover,
            "active" => NonTSPseudoClass::Active,
            "focus" => NonTSPseudoClass::Focus,
            "focus-within" => NonTSPseudoClass::FocusWithin,
            "focus-visible" => NonTSPseudoClass::FocusVisible,
            "visited" => NonTSPseudoClass::Visited,
            "link" => NonTSPseudoClass::Link,
            "any-link" => NonTSPseudoClass::AnyLink,
            "target" => NonTSPseudoClass::Target,
            "enabled" => NonTSPseudoClass::Enabled,
            "disabled" => NonTSPseudoClass::Disabled,
            "checked" => NonTSPseudoClass::Checked,
            _ => {
                return Err(location.new_custom_error(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ))
            }
        })
    }

    /// `::before`/`::after`/`::first-letter`に対応する。`::first-line`は非対応
    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<PseudoElement, cssparser::ParseError<'i, Self::Error>> {
        Ok(match_ignore_ascii_case! { &name,
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            "first-letter" => PseudoElement::FirstLetter,
            _ => {
                return Err(location.new_custom_error(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ))
            }
        })
    }
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

/// 非構造的(状態依存)擬似クラス。PDFは非対話的な出力なので、これらはいずれも
/// パースには対応しつつ、実際のマッチングでは常に非マッチとして扱う
/// ([`super::element_ref::ElementRef::match_non_ts_pseudo_class`]参照)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonTSPseudoClass {
    Hover,
    Active,
    Focus,
    FocusWithin,
    FocusVisible,
    Visited,
    Link,
    AnyLink,
    Target,
    Enabled,
    Disabled,
    Checked,
}

impl parser::NonTSPseudoClass for NonTSPseudoClass {
    type Impl = SgSelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        matches!(self, Self::Active | Self::Hover)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(
            self,
            Self::Active | Self::Hover | Self::Focus | Self::FocusWithin | Self::FocusVisible
        )
    }
}

impl ToCss for NonTSPseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            Self::Hover => ":hover",
            Self::Active => ":active",
            Self::Focus => ":focus",
            Self::FocusWithin => ":focus-within",
            Self::FocusVisible => ":focus-visible",
            Self::Visited => ":visited",
            Self::Link => ":link",
            Self::AnyLink => ":any-link",
            Self::Target => ":target",
            Self::Enabled => ":enabled",
            Self::Disabled => ":disabled",
            Self::Checked => ":checked",
        })
    }
}

/// 擬似要素。`::before`/`::after`(`content`宣言と組み合わせた生成コンテンツ)・
/// `::first-letter`(限定的なプロパティのみの上書きスタイル)に対応する。
/// `::first-line`は非対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    FirstLetter,
}

impl parser::PseudoElement for PseudoElement {
    type Impl = SgSelectorImpl;

    fn is_before_or_after(&self) -> bool {
        matches!(self, Self::Before | Self::After)
    }
}

impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            Self::Before => "::before",
            Self::After => "::after",
            Self::FirstLetter => "::first-letter",
        })
    }
}
