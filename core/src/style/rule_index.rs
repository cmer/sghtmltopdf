//! スタイルルールの索引。
//!
//! セレクタマッチングは要素ごとに行うため、素直に実装すると
//! 「要素数 x ルール数」回の照合になる。実際にはほとんどのルールは
//! 末尾の複合セレクタ(`div p`なら`p`の部分)にタグ名・クラス・idの
//! いずれかを要求しており、それを持たない要素とは照合するまでもなく
//! 非マッチと分かる。ここではルールをその要求ごとにバケットへ分け、
//! 要素が持つタグ名・クラス・idに対応するバケットだけを候補として
//! 返すことで、要素あたりの照合回数を候補数まで減らす。
//!
//! 索引が返すのはあくまで候補であり、実際にマッチするかの判定は
//! 従来どおり`selectors`の照合が行う(索引は取りこぼしを作らない)。

use std::collections::HashMap;

use selectors::parser::Component;

use crate::html::{Dom, NodeData, NodeId};

use super::selector_impl::SgSelectorImpl;
use super::stylesheet::StyleRule;

/// ルールが要求する、末尾の複合セレクタの絞り込みキー。
enum Bucket {
    Id(String),
    Class(String),
    LocalName(String),
    /// タグ名・クラス・idのいずれも要求しない(`*`、属性セレクタ、
    /// `:is()`など)。どの要素に対しても候補になる。
    Any,
}

#[derive(Debug, Default)]
pub struct RuleIndex {
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    by_local_name: HashMap<String, Vec<u32>>,
    /// 絞り込めないルール(常に候補になる)。
    any: Vec<u32>,
    /// 擬似要素セレクタを持つルール。通常のマッチングでは決してマッチ
    /// しないため上のバケットには入れず、擬似要素向けのマッチングだけが使う。
    pseudo: Vec<u32>,
    /// 索引を作った時点のルール数([`super::stylesheet::Stylesheet::index`]が
    /// 作り直しの要否を判断するために使う)。
    rule_count: usize,
}

impl RuleIndex {
    pub fn build(rules: &[StyleRule]) -> Self {
        let mut index = Self {
            rule_count: rules.len(),
            ..Self::default()
        };
        for (rule_index, rule) in rules.iter().enumerate() {
            let rule_index = rule_index as u32;
            for selector in rule.selectors.slice() {
                if selector.has_pseudo_element() {
                    push_unique(&mut index.pseudo, rule_index);
                    continue;
                }
                match bucket_of(selector) {
                    Bucket::Id(name) => {
                        push_unique(index.by_id.entry(name).or_default(), rule_index)
                    }
                    Bucket::Class(name) => {
                        push_unique(index.by_class.entry(name).or_default(), rule_index)
                    }
                    Bucket::LocalName(name) => {
                        push_unique(index.by_local_name.entry(name).or_default(), rule_index)
                    }
                    Bucket::Any => push_unique(&mut index.any, rule_index),
                }
            }
        }
        index
    }

    /// 索引を作った時点のルール数。
    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    /// `element`にマッチしうるルールの番号を、ソース順の昇順で`out`へ入れる。
    pub fn candidates(&self, dom: &Dom, element: NodeId, out: &mut Vec<u32>) {
        out.clear();
        let NodeData::Element { name, attrs, .. } = &dom.node(element).data else {
            return;
        };
        out.extend_from_slice(&self.any);
        if let Some(rules) = self.by_local_name.get(&*name.local) {
            out.extend_from_slice(rules);
        }
        for attr in attrs {
            match &*attr.name.local {
                "id" => {
                    if let Some(rules) = self.by_id.get(&*attr.value) {
                        out.extend_from_slice(rules);
                    }
                }
                "class" => {
                    for class in attr.value.split_ascii_whitespace() {
                        if let Some(rules) = self.by_class.get(class) {
                            out.extend_from_slice(rules);
                        }
                    }
                }
                _ => {}
            }
        }
        // バケットをまたいで同じルールが入りうる(`h1, .lead`のような
        // セレクタリスト)ため、ソース順に整えたうえで重複を落とす。
        out.sort_unstable();
        out.dedup();
    }

    /// 擬似要素セレクタを持つルールの番号(ソース順の昇順)。
    pub fn pseudo_candidates(&self) -> &[u32] {
        &self.pseudo
    }
}

/// 末尾が同じバケットになるセレクタが1つのルールに複数あっても、
/// ルールは1回だけ候補に入れる(番号は昇順に積まれる)。
fn push_unique(rules: &mut Vec<u32>, rule_index: u32) {
    if rules.last() != Some(&rule_index) {
        rules.push(rule_index);
    }
}

/// `selector`の末尾の複合セレクタが要求する絞り込みキー。
/// id > クラス > タグ名の順に強い(絞り込みが効く)ものを選ぶ。
fn bucket_of(selector: &selectors::parser::Selector<SgSelectorImpl>) -> Bucket {
    let mut bucket = Bucket::Any;
    // `iter`は末尾の複合セレクタだけを返す(結合子の手前で止まる)。
    for component in selector.iter() {
        match component {
            Component::ID(name) => return Bucket::Id(name.0.to_string()),
            Component::Class(name) => bucket = Bucket::Class(name.0.to_string()),
            // 大文字を含む型セレクタはHTML以外の名前空間でのみ大小を区別
            // するため、素直に絞り込めない。候補を取りこぼさないよう、
            // 絞り込みの対象から外す(=`Bucket::Any`のままにする)。
            Component::LocalName(name)
                if matches!(bucket, Bucket::Any) && name.name == name.lower_name =>
            {
                bucket = Bucket::LocalName(name.lower_name.0.to_string());
            }
            _ => {}
        }
    }
    bucket
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{self, NodeData};
    use crate::style::parse_stylesheet;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    fn candidates_for(css: &str, source: &str, tag: &str) -> Vec<u32> {
        let sheet = parse_stylesheet(css);
        let index = RuleIndex::build(&sheet.rules);
        let dom = html::parse(source.as_bytes());
        let element = find(&dom, dom.document(), tag).expect("element not found");
        let mut out = Vec::new();
        index.candidates(&dom, element, &mut out);
        out
    }

    #[test]
    fn only_rules_requiring_the_elements_tag_are_candidates() {
        let out = candidates_for(
            "p { color: red; } div { color: blue; } span { color: green; }",
            "<p>text</p>",
            "p",
        );
        assert_eq!(out, vec![0]);
    }

    #[test]
    fn class_and_id_rules_are_picked_up_from_the_attributes() {
        let out = candidates_for(
            ".lead { color: red; } #main { color: blue; } .other { color: green; }",
            r#"<p id="main" class="lead">text</p>"#,
            "p",
        );
        assert_eq!(out, vec![0, 1]);
    }

    #[test]
    fn the_rightmost_compound_decides_the_bucket() {
        // `div p`は末尾が`p`なので、`p`を持つ要素の候補に入る。
        let out = candidates_for("div p { color: red; }", "<div><p>text</p></div>", "p");
        assert_eq!(out, vec![0]);

        // 同じルールは`div`自身の候補には入らない。
        let out = candidates_for("div p { color: red; }", "<div><p>text</p></div>", "div");
        assert!(out.is_empty());
    }

    #[test]
    fn rules_that_cannot_be_narrowed_are_always_candidates() {
        let out = candidates_for(
            "* { color: red; } [data-x] { color: blue; }",
            "<p>text</p>",
            "p",
        );
        assert_eq!(out, vec![0, 1]);
    }

    #[test]
    fn a_selector_list_puts_the_rule_in_every_matching_bucket_but_only_once() {
        let css = "h1, .lead, p { color: red; }";
        assert_eq!(candidates_for(css, "<h1>t</h1>", "h1"), vec![0]);
        assert_eq!(
            candidates_for(css, r#"<div class="lead">t</div>"#, "div"),
            vec![0]
        );
        // 末尾が`p`のセレクタとクラスのセレクタの両方に該当しても1回だけ。
        assert_eq!(
            candidates_for(css, r#"<p class="lead">t</p>"#, "p"),
            vec![0]
        );
    }

    #[test]
    fn pseudo_element_rules_are_kept_out_of_the_normal_buckets() {
        let sheet = parse_stylesheet("p::before { content: 'x'; } p { color: red; }");
        let index = RuleIndex::build(&sheet.rules);
        let dom = html::parse(b"<p>text</p>");
        let element = find(&dom, dom.document(), "p").expect("p not found");

        let mut out = Vec::new();
        index.candidates(&dom, element, &mut out);
        assert_eq!(out, vec![1]);
        assert_eq!(index.pseudo_candidates(), &[0]);
    }
}
