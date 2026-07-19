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
//! コンテナ自身がページをまたいで分割される場合でも、そのコンテナの背景・枠線は
//! 実際に子が配置された各ページごとに再現する(簡易的なボックスフラグメンテーション、
//! [`place_split`]参照)。CSS Fragmentation仕様の`break-before`/`-after`/`-inside`や
//! orphans/widowsのような「どこで分割するか」の制御はしないが、「すでに決まった
//! 分割位置で、コンテナの装飾をどう引き継ぐか」は以下の簡易規則に従う:
//! - 上マージン/枠線/パディングは最初のフラグメントのみに適用する
//! - 下マージン/枠線/パディングは最後のフラグメントのみに適用する
//! - 左右の枠線/パディングは全フラグメントに適用する
//! - 背景色は各フラグメントの実際の内容範囲に対してそれぞれ塗る
//!
//! ページをまたがず1ページに収まる部分木は、元の構造を保ったまま配置される。

use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::{Dom, NodeId};
use crate::style::ComputedStyle;

use super::block::{layout_document, LaidOutBox, LaidOutContent};
use super::box_tree::build_box_tree;
use super::geometry::{EdgeSizes, FragmentPosition, Layout, Rect};
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
            place_split(
                b,
                children,
                page_height,
                pages,
                cursor,
                |child, ph, ps, c| {
                    place_box(child, ph, ps, c);
                },
            );
            return;
        }
        LaidOutContent::Inline(lines) if lines.len() > 1 => {
            place_split(b, lines, page_height, pages, cursor, |line, ph, ps, c| {
                place_line(line, ph, ps, c);
            });
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

/// `b`が1ページに収まらないため、子要素(`items`、`place_one`で1つずつ配置)単位で
/// 分割配置する。分割後、`b`自身の背景・枠線を各ページの実際の内容範囲に対して
/// 再現する装飾フラグメントを追加で挿入する(モジュールdoc参照)。
///
/// `items`は`LaidOutBox`(ブロック子要素)または[`LineBox`](インライン行)のどちらか。
fn place_split<T>(
    b: &LaidOutBox,
    items: &[T],
    page_height: f32,
    pages: &mut Vec<Page>,
    cursor: &mut f32,
    place_one: impl Fn(&T, f32, &mut Vec<Page>, &mut f32),
) {
    let top_extra = b.layout.margin.top + b.layout.border.top + b.layout.padding.top;
    let bottom_extra = b.layout.padding.bottom + b.layout.border.bottom + b.layout.margin.bottom;

    // 最初のフラグメントの前に、コンテナ自身の上マージン/枠線/パディング分の
    // スペースを確保する(この余白がページの残りを超える極端なケースの調整は
    // 行わない: M1の機械的改ページの簡略化の範囲内)。
    *cursor += top_extra;

    struct Segment {
        page_index: usize,
        start_index: usize,
    }

    let mut current_page = pages.len() - 1;
    let mut segments = vec![Segment {
        page_index: current_page,
        start_index: pages[current_page].boxes.len(),
    }];

    for item in items {
        place_one(item, page_height, pages, cursor);
        let now_page = pages.len() - 1;
        if now_page != current_page {
            // 新しいページへ進んだ。今回作られたページは、この`b`の内容以外が
            // 割り込む余地がないため、先頭(index 0)から始まる。
            for p in (current_page + 1)..=now_page {
                segments.push(Segment {
                    page_index: p,
                    start_index: 0,
                });
            }
            current_page = now_page;
        }
    }

    // 呼び出し元(兄弟要素)のため、下マージン/枠線/パディング分もカーソルへ加算する。
    *cursor += bottom_extra;

    // 実際に内容が置かれたセグメントだけを残す(例: 最初の子がページ先頭で
    // 改ページを起こし、直前のページには何も置かれなかった場合など)。
    let valid: Vec<&Segment> = segments
        .iter()
        .filter(|s| pages[s.page_index].boxes.len() > s.start_index)
        .collect();

    let fragments: Vec<(usize, usize, LaidOutBox)> = valid
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let is_first = i == 0;
            let is_last = i == valid.len() - 1;
            let end_index = pages[seg.page_index].boxes.len();
            let (top, bottom) = extent_of(&pages[seg.page_index].boxes[seg.start_index..end_index]);
            let layout = fragment_layout(&b.layout, top, bottom, is_first, is_last);
            let decoration = LaidOutBox {
                node: b.node,
                layout,
                content: LaidOutContent::Blocks(Vec::new()),
            };
            (seg.page_index, seg.start_index, decoration)
        })
        .collect();

    for (page_index, insert_index, decoration) in fragments {
        pages[page_index].boxes.insert(insert_index, decoration);
    }
}

/// `boxes`に実際に配置された子孫の、ページ内相対座標での垂直方向の union extent
/// (マージンボックスの上端の最小値・下端の最大値)を求める。
fn extent_of(boxes: &[LaidOutBox]) -> (f32, f32) {
    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for b in boxes {
        let box_top =
            b.layout.content.y - b.layout.padding.top - b.layout.border.top - b.layout.margin.top;
        let box_bottom = box_top + b.layout.margin_box_height();
        top = top.min(box_top);
        bottom = bottom.max(box_bottom);
    }
    (top, bottom)
}

/// コンテナ`original`のうち、1フラグメント分の装飾(背景・枠線)を描画するための
/// [`Layout`]を組み立てる。`content_y`/`content_bottom`はそのフラグメントの
/// コンテンツ領域の範囲(`is_first`なら上端はすでに`content_y`で確定済み、
/// `is_last`なら下端はまだ`padding-bottom`/`border-bottom`を含んでいない)。
///
/// `fragment`(→[`FragmentPosition`])には、`is_first`/`is_last`から求めた
/// 断片の位置を記録する。`border-radius`は計算スタイル側の値をそのまま使うため
/// (`Layout`は太さしか持たない)、レンダラ側([`crate::pdf::document`])が
/// この情報を見て「継続中の断片では角を丸めない」よう判断する。
fn fragment_layout(
    original: &Layout,
    content_y: f32,
    content_bottom: f32,
    is_first: bool,
    is_last: bool,
) -> Layout {
    let top_border = if is_first { original.border.top } else { 0.0 };
    let bottom_border = if is_last { original.border.bottom } else { 0.0 };
    let top_padding = if is_first { original.padding.top } else { 0.0 };
    let bottom_padding = if is_last {
        original.padding.bottom
    } else {
        0.0
    };
    let fragment = match (is_first, is_last) {
        (true, true) => FragmentPosition::Whole,
        (true, false) => FragmentPosition::First,
        (false, true) => FragmentPosition::Last,
        (false, false) => FragmentPosition::Middle,
    };

    Layout {
        content: Rect {
            x: original.content.x,
            y: content_y,
            width: original.content.width,
            height: (content_bottom - content_y).max(0.0),
        },
        padding: EdgeSizes {
            top: top_padding,
            right: original.padding.right,
            bottom: bottom_padding,
            left: original.padding.left,
        },
        border: EdgeSizes {
            top: top_border,
            right: original.border.right,
            bottom: bottom_border,
            left: original.border.left,
        },
        margin: EdgeSizes::default(),
        fragment,
    }
}

fn place_line(line: &LineBox, page_height: f32, pages: &mut Vec<Page>, cursor: &mut f32) {
    if *cursor > 0.0 && *cursor + line.rect.height > page_height {
        new_page(pages, cursor);
    }

    let shift = line.rect.y - *cursor;
    let mut translated = line.clone();
    translated.rect.y -= shift;

    let fragment = LaidOutBox {
        node: None,
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
        LaidOutContent::Table(rows) => {
            for row in rows.iter_mut() {
                for cell in row.cells.iter_mut() {
                    *cell = shift_box_y(cell, delta);
                }
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
        // 分割されていないボックスは`Whole`(border-radiusを全角に適用してよい)。
        assert_eq!(pages[0].boxes[0].layout.fragment, FragmentPosition::Whole);
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

    /// `page.boxes`の中から、`target`をnodeに持ち、かつ`LaidOutContent::Blocks(vec![])`
    /// (=装飾専用フラグメント)であるものを探す。
    fn find_decoration_fragment(page: &Page, target: NodeId) -> Option<&LaidOutBox> {
        page.boxes.iter().find(|b| {
            b.node == Some(target)
                && matches!(&b.content, LaidOutContent::Blocks(c) if c.is_empty())
        })
    }

    #[test]
    fn split_container_gets_a_decoration_fragment_on_every_page_it_spans() {
        let mut html_src = String::from(r#"<div class="wrapper">"#);
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".wrapper { border: 2px solid black; padding: 5px; margin: 0; } \
             .item { height: 100px; margin: 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(
            pages.len() >= 3,
            "expected the wrapper to span at least 3 pages, got {}",
            pages.len()
        );

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let wrapper = divs[0];

        // すべてのページに、wrapperの装飾フラグメントが存在するはず
        // (背景・枠線がページをまたいでも失われないことの確認)。
        let decorations: Vec<&LaidOutBox> = pages
            .iter()
            .map(|page| {
                find_decoration_fragment(page, wrapper)
                    .expect("every page the wrapper spans should carry a decoration fragment")
            })
            .collect();

        // 最初のフラグメントだけが上枠線・上パディングを持つ。
        assert_eq!(decorations[0].layout.border.top, 2.0);
        assert_eq!(decorations[0].layout.padding.top, 5.0);
        assert_eq!(decorations[0].layout.fragment, FragmentPosition::First);
        // 最後のフラグメントだけが下枠線・下パディングを持つ。
        let last = decorations.last().unwrap();
        assert_eq!(last.layout.border.bottom, 2.0);
        assert_eq!(last.layout.padding.bottom, 5.0);
        assert_eq!(last.layout.fragment, FragmentPosition::Last);
        // 中間のフラグメントは`Middle`(border-radiusの角丸抑制に使う)。
        for decoration in &decorations[1..decorations.len() - 1] {
            assert_eq!(decoration.layout.fragment, FragmentPosition::Middle);
        }

        // 左右の枠線・パディングは全フラグメントに適用される。
        for decoration in &decorations {
            assert_eq!(decoration.layout.border.left, 2.0);
            assert_eq!(decoration.layout.padding.left, 5.0);
            assert!(decoration.layout.content.height > 0.0);
        }

        // 中間のフラグメント(最初でも最後でもないもの)は上下の枠線・パディングを持たない。
        for decoration in &decorations[1..decorations.len() - 1] {
            assert_eq!(decoration.layout.border.top, 0.0);
            assert_eq!(decoration.layout.border.bottom, 0.0);
            assert_eq!(decoration.layout.padding.top, 0.0);
            assert_eq!(decoration.layout.padding.bottom, 0.0);
        }

        // 中身の<p>群は引き続きすべて見つかるはず(装飾フラグメント追加による
        // 既存の子配置ロジックへの副作用がないことの回帰確認)。
        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        for &p_id in &ps {
            let found = pages
                .iter()
                .any(|page| page.boxes.iter().any(|b| box_contains_node(b, p_id)));
            assert!(found, "p {p_id:?} should still be placed on some page");
        }
    }

    #[test]
    fn split_container_without_border_or_padding_still_gets_zero_sized_decoration() {
        // 枠線もパディングもない場合でも、装飾フラグメント自体は追加されるが
        // (背景色の有無をpaginateモジュールは判断しないため)、見た目には
        // 一切影響しない(render_box側でbackground_color.alpha==0かつ
        // border-style: noneなら何も描かれない)。
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

        let mut divs = Vec::new();
        find_all(&dom, dom.document(), "div", &mut divs);
        let wrapper = divs[0];

        for page in &pages {
            if let Some(decoration) = find_decoration_fragment(page, wrapper) {
                assert_eq!(decoration.layout.border.top, 0.0);
                assert_eq!(decoration.layout.padding.top, 0.0);
            }
        }
    }
}
