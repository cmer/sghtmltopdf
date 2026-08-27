//! `@layer`(カスケードレイヤー)のE2Eテスト(#20)。
//!
//! Tailwind v4は出力全体を`@layer`ブロックで包むため、ブロックを捨てると
//! 文書が丸ごと無装飾になる。レイヤーの優先順位は実装せず、中のルールを
//! 書かれた順にトップレベルへ展開する。
//!
//! `custom_properties.rs`と同じ方針: `<style>`からの抽出→カスケード→レイアウト
//! という実際の経路を通して回帰を検知する。

use std::path::PathBuf;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::img::{DocumentImageCache, ImageFetcher};
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, LaidOutBox, LaidOutContent, PageSettings,
};
use sghtmltopdf_core::style::{compute_styles, extract_author_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn test_fonts() -> FontCollection {
    FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ])
}

fn find_all_tags(dom: &Dom, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == tag {
            out.push(id);
        }
    }
    for child in dom.children(id) {
        find_all_tags(dom, child, tag, out);
    }
}

fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
    if b.node == Some(target) {
        return Some(b);
    }
    if let LaidOutContent::Blocks(children) = &b.content {
        for child in children {
            if let Some(found) = find_laid_out(child, target) {
                return Some(found);
            }
        }
    }
    None
}

fn layout(html_body: &str, css: &str) -> (Dom, LaidOutBox) {
    let dom = html::parse(
        format!("<html><head><style>{css}</style></head><body>{html_body}</body></html>")
            .as_bytes(),
    );
    let fetcher = ImageFetcher::new(PathBuf::from("."), false);
    let cache = DocumentImageCache::new();
    let author = extract_author_stylesheet(&dom, &fetcher, &cache);
    let ua = user_agent_stylesheet();
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let tree = build_box_tree(&dom, &styles);
    let laid = layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    );
    (dom, laid)
}

/// issue #20の再現手順そのもの。`.probe { margin-left: 90px }`を素のまま、
/// `@layer utilities { }`で包んで、`@layer base, utilities;`の後に置いて、
/// の3通りで`X`の左端が同じ位置に来ること。
fn probe_x(css: &str) -> f32 {
    let (dom, laid) = layout(
        r#"<div class="probe">X</div>"#,
        &format!("* {{ margin: 0; padding: 0 }} {css}"),
    );
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    find_laid_out(&laid, divs[0]).unwrap().layout.content.x
}

#[test]
fn rule_inside_a_layer_block_is_applied() {
    let plain = probe_x(".probe { margin-left: 90px }");
    assert_eq!(plain, 90.0);
    assert_eq!(
        probe_x("@layer utilities { .probe { margin-left: 90px } }"),
        plain,
        "a rule wrapped in @layer must be applied like a plain rule"
    );
}

#[test]
fn layer_statement_does_not_poison_subsequent_rules() {
    assert_eq!(
        probe_x("@layer base, utilities; .probe { margin-left: 90px }"),
        90.0
    );
}

/// Tailwind v4の出力の形(順序宣言+複数のレイヤーブロック+`:root`の
/// カスタムプロパティ+ネストした`@media`)を縮めたもの。`var()`の
/// テキスト置換が`@layer`の中でも効くことを含めて確認する。
#[test]
fn tailwind_v4_shaped_bundle_is_applied() {
    let css = r#"
        @layer theme, base, components, utilities;
        @layer theme {
          :root { --spacing: 0.25rem; --color-red: rgb(255, 0, 0); }
        }
        @layer base {
          *, ::before, ::after { box-sizing: border-box; margin: 0; padding: 0; }
        }
        @layer components;
        @layer utilities {
          .ml-8 { margin-left: calc(var(--spacing) * 8); }
          .w-40 { width: calc(var(--spacing) * 40); }
          @media print { .print\:w-20 { width: calc(var(--spacing) * 20); } }
        }
    "#;
    let (dom, laid) = layout(r#"<div class="ml-8 w-40 print:w-20">X</div>"#, css);
    let mut divs = Vec::new();
    find_all_tags(&dom, dom.document(), "div", &mut divs);
    let div = find_laid_out(&laid, divs[0]).unwrap();
    // 0.25rem = 4px; ml-8 = 32px; print:w-20 = 80px(後勝ちでw-40の160pxを上書き)
    assert_eq!(div.layout.content.x, 32.0);
    assert_eq!(div.layout.content.width, 80.0);
}
