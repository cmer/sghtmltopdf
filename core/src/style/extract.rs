//! DOM中の`<style>`要素からauthorスタイルシートを組み立てる。
//!
//! `style="..."`属性(インラインスタイル)の抽出は未対応(T3参照)。

use crate::html::{Dom, NodeData, NodeId};

use super::stylesheet::{parse_stylesheet, Stylesheet};

/// DOM中の全ての`<style>`要素のテキスト内容を連結してパースする。
pub fn extract_author_stylesheet(dom: &Dom) -> Stylesheet {
    let mut css = String::new();
    collect_style_text(dom, dom.document(), &mut css);
    parse_stylesheet(&css)
}

fn collect_style_text(dom: &Dom, node: NodeId, out: &mut String) {
    if let NodeData::Element { name, .. } = &dom.node(node).data {
        if &*name.local == "style" {
            for child in dom.children(node) {
                if let NodeData::Text { contents } = &dom.node(child).data {
                    out.push_str(contents);
                    out.push('\n');
                }
            }
            return;
        }
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    #[test]
    fn extracts_and_parses_style_tag_contents() {
        let dom = html::parse(
            br#"<html><head><style>p { color: rgb(1, 2, 3); }</style></head>
                <body><p>text</p></body></html>"#,
        );
        let sheet = extract_author_stylesheet(&dom);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn combines_multiple_style_tags() {
        let dom = html::parse(
            br#"<html><head>
                <style>p { color: rgb(1, 2, 3); }</style>
                <style>div { color: rgb(4, 5, 6); }</style>
                </head><body></body></html>"#,
        );
        let sheet = extract_author_stylesheet(&dom);
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn returns_empty_stylesheet_when_no_style_tags() {
        let dom = html::parse(b"<p>text</p>");
        let sheet = extract_author_stylesheet(&dom);
        assert!(sheet.rules.is_empty());
    }
}
