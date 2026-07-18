//! レイアウト済みのボックス木を、ページ残り高さに基づいて機械的に分割する。
//!
//! `break-before`/`break-after`/`orphans`/`widows`といったCSS Fragmentation
//! 仕様のヒントは一切見ない(M2で対応)。ボックスがページに収まらない場合、
//! 以下の優先順で分割を試みる:
//! 1. ブロックコンテナなら、その子ボックス単位で置き直す
//! 2. 複数行のインラインコンテンツなら、行(line box)単位で分割する
//! 3. それでも分割できない最小単位(空の要素・1行のみの内容)は次ページの
//!    先頭にまるごと送る(1ページに収まらないほど巨大な場合はそのままはみ出す)
//!
//! コンテナ自身がページをまたいで分割される場合、そのコンテナの背景・枠線は
//! どちらのページにも再現されない(子だけがそれぞれのページに配置される)。
//! ページをまたがず1ページに収まる部分木は、元の構造を保ったまま配置される。

use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::{Dom, NodeId};
use crate::style::ComputedStyle;

use super::block::{layout_document, LaidOutBox, LaidOutContent};
use super::box_tree::build_box_tree;
use super::geometry::{Layout, Rect};
use super::inline::LineBox;
use super::page::PageSettings;

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub boxes: Vec<LaidOutBox>,
}

/// `root`(通常は[`super::layout_document`]の返り値)を、高さ`page_content_height`の
/// ページに分割する。
pub fn paginate(root: &LaidOutBox, page_content_height: f32) -> Vec<Page> {
    let mut pages = vec![Page::default()];
    let mut cursor = 0.0f32;
    place_box(root, page_content_height, &mut pages, &mut cursor);
    pages
}

/// DOM+計算スタイルから、ボックスツリー構築・レイアウト・ページ分割までを一括で行う。
pub fn paginate_document(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
) -> Vec<Page> {
    let tree = build_box_tree(dom, styles);
    let laid_out = layout_document(&tree, styles, fonts, settings.content_width());
    paginate(&laid_out, settings.content_height())
}

fn place_box(b: &LaidOutBox, page_height: f32, pages: &mut Vec<Page>, cursor: &mut f32) {
    let height = b.layout.margin_box_height();

    if *cursor + height <= page_height {
        place_leaf(b, pages, cursor);
        return;
    }

    match &b.content {
        LaidOutContent::Blocks(children) if !children.is_empty() => {
            for child in children {
                place_box(child, page_height, pages, cursor);
            }
            return;
        }
        LaidOutContent::Inline(lines) if lines.len() > 1 => {
            for line in lines {
                place_line(b.node, line, page_height, pages, cursor);
            }
            return;
        }
        _ => {}
    }

    // これ以上分割できない最小単位。ページに余白を使ってしまっていれば
    // 次ページの先頭へ送る(まっさらなページの先頭ならそのまま置く)。
    if *cursor > 0.0 {
        new_page(pages, cursor);
    }
    place_leaf(b, pages, cursor);
}

fn place_line(
    node: Option<NodeId>,
    line: &LineBox,
    page_height: f32,
    pages: &mut Vec<Page>,
    cursor: &mut f32,
) {
    if *cursor > 0.0 && *cursor + line.rect.height > page_height {
        new_page(pages, cursor);
    }

    let shift = line.rect.y - *cursor;
    let mut translated = line.clone();
    translated.rect.y -= shift;

    let fragment = LaidOutBox {
        node,
        layout: Layout {
            content: translated.rect,
            ..Layout::default()
        },
        content: LaidOutContent::Inline(vec![translated]),
    };
    *cursor += line.rect.height;
    pages
        .last_mut()
        .expect("paginateは常に1ページ以上を保持する")
        .boxes
        .push(fragment);
}

fn place_leaf(b: &LaidOutBox, pages: &mut [Page], cursor: &mut f32) {
    let margin_box_top =
        b.layout.content.y - b.layout.padding.top - b.layout.border.top - b.layout.margin.top;
    let shift = margin_box_top - *cursor;
    let translated = shift_box_y(b, shift);
    let height = b.layout.margin_box_height();

    *cursor += height;
    pages
        .last_mut()
        .expect("paginateは常に1ページ以上を保持する")
        .boxes
        .push(translated);
}

fn new_page(pages: &mut Vec<Page>, cursor: &mut f32) {
    pages.push(Page::default());
    *cursor = 0.0;
}

/// `b`の部分木全体のY座標を`delta`だけ平行移動した複製を返す
/// (1ページ全体の連続座標から、ページ内相対座標への変換に使う)。
fn shift_box_y(b: &LaidOutBox, delta: f32) -> LaidOutBox {
    let mut b = b.clone();
    shift_rect_y(&mut b.layout.content, delta);

    match &mut b.content {
        LaidOutContent::Blocks(children) => {
            for child in children.iter_mut() {
                *child = shift_box_y(child, delta);
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines.iter_mut() {
                shift_rect_y(&mut line.rect, delta);
            }
        }
    }

    b
}

fn shift_rect_y(rect: &mut Rect, delta: f32) {
    rect.y -= delta;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Font;
    use crate::html::{self, Dom, NodeData};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(TEST_FONT_PATH).expect("should load bundled test font")
        ])
    }

    fn find_all(dom: &Dom, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                out.push(id);
            }
        }
        for child in dom.children(id) {
            find_all(dom, child, tag, out);
        }
    }

    fn box_contains_node(b: &LaidOutBox, target: NodeId) -> bool {
        if b.node == Some(target) {
            return true;
        }
        if let LaidOutContent::Blocks(children) = &b.content {
            return children
                .iter()
                .any(|child| box_contains_node(child, target));
        }
        false
    }

    /// ページ内のボックスがページ高さの範囲内(多少の誤差を許容)に収まっているか
    /// を再帰的に確認する。
    fn assert_within_page(b: &LaidOutBox, page_height: f32) {
        let top =
            b.layout.content.y - b.layout.padding.top - b.layout.border.top - b.layout.margin.top;
        assert!(top >= -0.01, "box top {top} should not be negative");
        assert!(
            top + b.layout.margin_box_height() <= page_height + 0.01,
            "box bottom should not exceed page height {page_height}"
        );
        if let LaidOutContent::Blocks(children) = &b.content {
            for child in children {
                assert_within_page(child, page_height);
            }
        }
    }

    #[test]
    fn page_settings_computes_content_area() {
        let settings = PageSettings::default();
        assert_eq!(
            settings.content_width(),
            settings.size.width - settings.margin.left - settings.margin.right
        );
        assert_eq!(
            settings.content_height(),
            settings.size.height - settings.margin.top - settings.margin.bottom
        );
    }

    #[test]
    fn short_document_fits_on_a_single_page_and_keeps_structure() {
        let dom = html::parse(br#"<p>hello</p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(pages.len(), 1);

        // 分割が発生しないため、無名ルート(node: None)ごと元の構造が保たれているはず。
        let mut htmls = Vec::new();
        find_all(&dom, dom.document(), "html", &mut htmls);
        assert_eq!(pages[0].boxes.len(), 1);
        assert_eq!(pages[0].boxes[0].node, None);
        assert!(box_contains_node(&pages[0].boxes[0], htmls[0]));
    }

    #[test]
    fn tall_content_distributes_across_multiple_pages_without_losing_items() {
        let mut html_src = String::from("<div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".item { height: 100px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(
            pages.len() > 1,
            "20 items of 100px should overflow a single page"
        );

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        assert_eq!(ps.len(), 20);
        for &p_id in &ps {
            let found_on_some_page = pages
                .iter()
                .any(|page| page.boxes.iter().any(|b| box_contains_node(b, p_id)));
            assert!(
                found_on_some_page,
                "p {p_id:?} should be placed on some page"
            );
        }

        for page in &pages {
            for b in &page.boxes {
                assert_within_page(b, settings.content_height());
            }
        }
    }

    #[test]
    fn long_paragraph_splits_across_pages_by_line() {
        let words: Vec<String> = (0..1000).map(|i| format!("word{i}")).collect();
        let html_src = format!("<p>{}</p>", words.join(" "));
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(
            pages.len() > 1,
            "1000 words should wrap into more lines than fit on one page"
        );

        let total_lines: usize = pages
            .iter()
            .flat_map(|page| &page.boxes)
            .filter_map(|b| match &b.content {
                LaidOutContent::Inline(lines) => Some(lines.len()),
                _ => None,
            })
            .sum();
        assert!(
            total_lines > 20,
            "1000 words should wrap into many lines total, got {total_lines}"
        );
        assert!(
            pages[0].boxes.len() > 1,
            "first page should hold multiple line fragments, got {}",
            pages[0].boxes.len()
        );
    }
}
