//! セレクタマッチングと、カスケード順(オリジン→詳細度→ソース順)による
//! 適用宣言の並べ替え。
//!
//! 同じプロパティが複数の宣言で指定された場合にどれを採用するか(継承含む)は
//! 計算スタイル側(スタイル計算フェーズ)の責務とする。ここでは、後ろにあるものほど
//! 優先度が高くなるよう順序付けた宣言列を返すところまでを行う。

use selectors::matching::{
    self, MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::parser::SelectorList;

use crate::html::{Dom, NodeId};

use super::element_ref::ElementRef;
use super::properties::PropertyDeclaration;
use super::selector_impl::SgSelectorImpl;
use super::stylesheet::Stylesheet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    UserAgent,
    Author,
}

/// `dom`上の`element`に適用される宣言を、カスケード優先度の昇順
/// (先頭が最も優先度が低く、末尾が最も優先度が高い)で返す。
pub fn matching_declarations<'a>(
    dom: &Dom,
    element: NodeId,
    ua: &'a Stylesheet,
    author: &'a Stylesheet,
) -> Vec<&'a PropertyDeclaration> {
    let el = ElementRef::new(dom, element);
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );

    let mut matched: Vec<(Origin, u32, usize, &'a Vec<PropertyDeclaration>)> = Vec::new();

    for (origin, sheet) in [(Origin::UserAgent, ua), (Origin::Author, author)] {
        for (source_order, rule) in sheet.rules.iter().enumerate() {
            if let Some(specificity) = best_matching_specificity(&rule.selectors, &el, &mut context)
            {
                matched.push((origin, specificity, source_order, &rule.declarations));
            }
        }
    }

    matched.sort_by_key(|(origin, specificity, source_order, _)| {
        (*origin, *specificity, *source_order)
    });

    matched
        .into_iter()
        .flat_map(|(_, _, _, declarations)| declarations.iter())
        .collect()
}

/// リスト中、要素に実際にマッチしたセレクタの中で最大の詳細度を返す
/// (`h1, h2 { ... }`のようなセレクタグループは、マッチした方の詳細度を使う)。
fn best_matching_specificity(
    selectors: &SelectorList<SgSelectorImpl>,
    element: &ElementRef,
    context: &mut MatchingContext<SgSelectorImpl>,
) -> Option<u32> {
    selectors
        .slice()
        .iter()
        .filter(|selector| matching::matches_selector(selector, 0, None, element, context))
        .map(|selector| selector.specificity())
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{self, NodeData};
    use crate::style::{parse_stylesheet, Color, Display};

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    fn last_color(decls: &[&PropertyDeclaration]) -> Option<Color> {
        decls.iter().rev().find_map(|d| match d {
            PropertyDeclaration::Color(c) => Some(*c),
            _ => None,
        })
    }

    fn last_display(decls: &[&PropertyDeclaration]) -> Option<Display> {
        decls.iter().rev().find_map(|d| match d {
            PropertyDeclaration::Display(display) => Some(*display),
            _ => None,
        })
    }

    fn rgb(v: u8) -> Color {
        Color::Rgba {
            red: v,
            green: v,
            blue: v,
            alpha: 1.0,
        }
    }

    #[test]
    fn specificity_beats_source_order() {
        let dom = html::parse(br#"<div id="x" class="c">t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        // 詳細度の高い#xルールをソース上は先頭に書いても、
        // 一番優先されるのは詳細度なので最後に並ぶはず。
        let author = parse_stylesheet(
            "#x { color: rgb(2, 2, 2); } .c { color: rgb(1, 1, 1); } div { color: rgb(0, 0, 0); }",
        );
        let ua = Stylesheet::default();

        let decls = matching_declarations(&dom, div, &ua, &author);
        assert_eq!(last_color(&decls), Some(rgb(2)));
    }

    #[test]
    fn later_source_order_wins_on_specificity_tie() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let author = parse_stylesheet("div { color: rgb(9, 9, 9); } div { color: rgb(8, 8, 8); }");
        let ua = Stylesheet::default();

        let decls = matching_declarations(&dom, div, &ua, &author);
        assert_eq!(last_color(&decls), Some(rgb(8)));
    }

    #[test]
    fn author_origin_beats_user_agent_on_specificity_tie() {
        let dom = html::parse(br#"<div>t</div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        let ua = parse_stylesheet("div { display: block; }");
        let author = parse_stylesheet("div { display: inline; }");

        let decls = matching_declarations(&dom, div, &ua, &author);
        assert_eq!(last_display(&decls), Some(Display::Inline));
    }

    #[test]
    fn descendant_combinator_matches_nested_element() {
        let dom = html::parse(br#"<div><p>inner</p></div><p>outer</p>"#);
        let ps: Vec<_> = {
            let mut out = Vec::new();
            fn collect(dom: &Dom, id: NodeId, out: &mut Vec<NodeId>) {
                if let NodeData::Element { name, .. } = &dom.node(id).data {
                    if &*name.local == "p" {
                        out.push(id);
                    }
                }
                for child in dom.children(id) {
                    collect(dom, child, out);
                }
            }
            collect(&dom, dom.document(), &mut out);
            out
        };
        assert_eq!(ps.len(), 2, "expected both <p> elements to be found");

        let author = parse_stylesheet("div p { color: rgb(3, 3, 3); }");
        let ua = Stylesheet::default();

        let inner_decls = matching_declarations(&dom, ps[0], &ua, &author);
        let outer_decls = matching_declarations(&dom, ps[1], &ua, &author);

        assert_eq!(last_color(&inner_decls), Some(rgb(3)));
        assert_eq!(last_color(&outer_decls), None);
    }
}
