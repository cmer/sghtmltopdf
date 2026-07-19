//! レイアウト済みのボックス木を、ページ残り高さに基づいて分割する。
//!
//! `break-before`/`break-after`/`break-inside`(`orphans`/`widows`はM2の別段階
//! [T15]で対応)を尊重しつつ、ボックスがページに収まらない場合は以下の優先順で
//! 分割を試みる:
//! 1. `break-inside: avoid`かつ丸ごと1ページに収まる大きさなら、分割せず
//!    次ページの先頭へまるごと送る
//! 2. ブロックコンテナなら、その子ボックス単位で置き直す(各子の
//!    `break-before`/`break-after: always`もこの単位で強制改ページとして働く)
//! 3. 複数行のインラインコンテンツなら、行(line box)単位で分割する
//! 4. それでも分割できない最小単位(空の要素・1行のみの内容)は次ページの
//!    先頭にまるごと送る(1ページに収まらないほど巨大な場合はそのままはみ出す)
//!
//! 子孫の`break-before`/`break-after: always`は、祖先の部分木がページ残り高さに
//! 収まる場合でも見逃してはならない(強制改ページはオーバーフローとは独立した
//! 明示的な指定のため)。[`subtree_requires_child_walk`]が、部分木内にこうした
//! 強制改ページが存在するかを事前に判定し、存在すれば「丸ごと1個のリーフとして
//! 配置する」高速経路を使わずに子要素単位の配置へフォールバックする。
//!
//! コンテナ自身がページをまたいで分割される場合でも、そのコンテナの背景・枠線は
//! 実際に子が配置された各ページごとに再現する(簡易的なボックスフラグメンテーション、
//! [`place_split`]参照)。「すでに決まった分割位置で、コンテナの装飾をどう
//! 引き継ぐか」は以下の簡易規則に従う:
//! - 上マージン/枠線/パディングは最初のフラグメントのみに適用する
//! - 下マージン/枠線/パディングは最後のフラグメントのみに適用する
//! - 左右の枠線/パディングは全フラグメントに適用する
//! - 背景色は各フラグメントの実際の内容範囲に対してそれぞれ塗る
//!
//! ページをまたがず1ページに収まり、かつ強制改ページも内包しない部分木は、
//! 元の構造を保ったまま配置される。

use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::{Dom, NodeId};
use crate::style::{BreakBetween, BreakInside, ComputedStyle};

use super::block::{layout_document, FragmentationHints, LaidOutBox, LaidOutContent};
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
    let has_forced_break_inside = subtree_requires_child_walk(b);

    if *cursor + height <= page_height && !has_forced_break_inside {
        place_leaf(b, pages, cursor);
        return;
    }

    // `break-inside: avoid`: 丸ごと(空の)1ページに収まる大きさで、かつ内部に
    // 強制改ページを内包しない場合は、分割せず次ページの先頭へまるごと送る。
    // 1ページに収まらないほど巨大な場合はこの限りではなく、best-effortで
    // 通常通り分割する(無限ループ・出力不能を避けるための例外)。
    // 現在のページに実際の内容が何もなければ(祖先のマージン分だけ`cursor`が
    // 進んでいるだけの場合を含む)、移動しても無意味なのでそのまま現在の
    // ページに置く。
    if b.fragmentation.break_inside == BreakInside::Avoid
        && current_page_has_content(pages)
        && height <= page_height
        && !has_forced_break_inside
    {
        new_page(pages, cursor);
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
                |child: &LaidOutBox| {
                    (
                        child.fragmentation.break_before == BreakBetween::Always,
                        child.fragmentation.break_after == BreakBetween::Always,
                    )
                },
                |child, ph, ps, c| {
                    place_box(child, ph, ps, c);
                },
            );
            return;
        }
        LaidOutContent::Inline(lines) if lines.len() > 1 => {
            place_split(
                b,
                lines,
                page_height,
                pages,
                cursor,
                // 行(line box)は`break-before`/`break-after`を持たない
                // (これらはブロックレベルの兄弟間にのみ意味を持つ)。
                |_| (false, false),
                |line, ph, ps, c| {
                    place_line(line, ph, ps, c);
                },
            );
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

/// `b`の部分木内(ブロックの子孫のみ、インライン行・テーブル内部は対象外)に、
/// `break-before`/`break-after: always`を持つボックスが存在するかどうか。
///
/// これが`true`の場合、`b`自身がページ残り高さに収まっていても「丸ごと1個の
/// リーフとして配置する」高速経路は使えない(強制改ページの位置を見逃して
/// しまうため)。テーブルの内部行・インライン行の分割はM2のスコープ外
/// (`0001-rest-of-m1.md`参照)なので、`Blocks`のみ再帰する。
fn subtree_requires_child_walk(b: &LaidOutBox) -> bool {
    match &b.content {
        LaidOutContent::Blocks(children) => children.iter().any(|child| {
            child.fragmentation.break_before == BreakBetween::Always
                || child.fragmentation.break_after == BreakBetween::Always
                || subtree_requires_child_walk(child)
        }),
        LaidOutContent::Inline(_) | LaidOutContent::Table(_) => false,
    }
}

/// `b`が1ページに収まらない(または内部に強制改ページを内包する)ため、子要素
/// (`items`、`place_one`で1つずつ配置)単位で分割配置する。分割後、`b`自身の
/// 背景・枠線を各ページの実際の内容範囲に対して再現する装飾フラグメントを
/// 追加で挿入する(モジュールdoc参照)。
///
/// `items`は`LaidOutBox`(ブロック子要素)または[`LineBox`](インライン行)のどちらか。
/// `break_hints`は各要素について`(直前に強制改ページが必要か, 直後に強制改ページが
/// 必要か)`を返すコールバック(行には`break-before`/`break-after`の概念がないため、
/// 呼び出し元は常に`(false, false)`を返すコールバックを渡す)。
fn place_split<T>(
    b: &LaidOutBox,
    items: &[T],
    page_height: f32,
    pages: &mut Vec<Page>,
    cursor: &mut f32,
    break_hints: impl Fn(&T) -> (bool, bool),
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

    // 強制改ページ(`break-before`/`break-after: always`)発生時、新しいページを
    // 開始し対応するセグメントを追加する共通処理。オーバーフローによる自然な
    // ページ送りとの二重計上を避けるため、`current_page`もその場で更新する。
    let force_new_page = |pages: &mut Vec<Page>,
                          cursor: &mut f32,
                          current_page: &mut usize,
                          segments: &mut Vec<Segment>| {
        new_page(pages, cursor);
        *current_page = pages.len() - 1;
        segments.push(Segment {
            page_index: *current_page,
            start_index: 0,
        });
    };

    for (i, item) in items.iter().enumerate() {
        let (breaks_before, breaks_after) = break_hints(item);
        // 現在のページに実際の内容が何もなければ(祖先のマージン分だけ`cursor`が
        // 進んでいるだけの場合を含む)、改ページしても無意味な空ページを
        // 作るだけなので何もしない。
        if breaks_before && current_page_has_content(pages) {
            force_new_page(pages, cursor, &mut current_page, &mut segments);
        }

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

        // 次に置く要素がある場合のみ改ページする(末尾の要素の後ろに
        // 空ページを作らないため)。
        if breaks_after && i + 1 < items.len() {
            force_new_page(pages, cursor, &mut current_page, &mut segments);
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
                // 装飾専用フラグメントはこれ以上分割対象にならないため、
                // fragmentationヒントは意味を持たない(初期値のまま)。
                fragmentation: FragmentationHints::default(),
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
        // 1行だけの合成ラッパーボックスなので、fragmentationヒントは持たない
        // (orphans/widowsの判断は呼び出し元(`place_split`)が行数単位で行う)。
        fragmentation: FragmentationHints::default(),
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

/// 現在のページに実際に配置されたボックスが1つでもあるか。`cursor`は祖先の
/// マージン/枠線/パディング分だけ既に進んでいることがあるため(まだ何も
/// 描画されていなくても`cursor > 0.0`になり得る)、「強制改ページが本当に
/// 意味のある移動か(=現在のページを空のまま捨てずに済むか)」の判定には
/// `cursor`ではなくこちらを使う。
fn current_page_has_content(pages: &[Page]) -> bool {
    !pages
        .last()
        .expect("paginateは常に1ページ以上を保持する")
        .boxes
        .is_empty()
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

    /// `target`をnodeに持ち、実際に内容を伴う(装飾専用フラグメントではない)
    /// ボックスがそのページにあるか。
    fn page_contains_content(page: &Page, target: NodeId) -> bool {
        page.boxes.iter().any(|b| box_contains_node(b, target))
    }

    #[test]
    fn break_before_always_forces_a_new_page_even_though_both_fit_on_one_page() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 50px; margin: 0; } \
             .b { height: 50px; margin: 0; break-before: always; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(
            pages.len(),
            2,
            "break-before: always should force a new page even though both \
             paragraphs easily fit on a single page"
        );

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let (a, b) = (ps[0], ps[1]);

        assert!(page_contains_content(&pages[0], a));
        assert!(!page_contains_content(&pages[0], b));
        assert!(page_contains_content(&pages[1], b));
        assert!(!page_contains_content(&pages[1], a));
    }

    #[test]
    fn break_before_always_on_the_first_element_does_not_create_a_blank_leading_page() {
        let dom = html::parse(br#"<p class="a">A</p>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".a { height: 50px; margin: 0; break-before: always; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(
            pages.len(),
            1,
            "break-before: always on the very first element of the document \
             should not produce a blank leading page"
        );
    }

    #[test]
    fn break_after_always_forces_a_new_page_before_the_next_sibling() {
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 50px; margin: 0; break-after: always; } \
             .b { height: 50px; margin: 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(pages.len(), 2);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        let (a, b) = (ps[0], ps[1]);

        assert!(page_contains_content(&pages[0], a));
        assert!(page_contains_content(&pages[1], b));
        assert!(!page_contains_content(&pages[0], b));
    }

    #[test]
    fn break_after_always_on_the_last_element_does_not_create_a_trailing_blank_page() {
        let dom = html::parse(br#"<p class="a">A</p>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".a { height: 50px; margin: 0; break-after: always; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(
            pages.len(),
            1,
            "break-after: always on the very last element should not produce \
             a trailing blank page"
        );
    }

    #[test]
    fn nested_break_before_is_honored_even_when_the_whole_subtree_fits_on_one_page() {
        // wrapper divの中身は合計しても(既定のページ高さに比べれば)ごく小さく、
        // 「丸ごと1個のリーフとして配置する」高速経路の対象になり得る。それでも
        // 内部のbには`break-before: always`があるので見逃してはならない。
        let dom = html::parse(br#"<div><p class="a">A</p><p class="b">B</p></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".a { height: 10px; margin: 0; } \
             .b { height: 10px; margin: 0; break-before: always; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(pages.len(), 2);

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        assert!(page_contains_content(&pages[0], ps[0]));
        assert!(page_contains_content(&pages[1], ps[1]));
    }

    #[test]
    fn break_inside_avoid_moves_the_whole_block_to_the_next_page_instead_of_splitting() {
        let settings = PageSettings::default();
        // fillerでページ残り高さを、wrapperの合計高さ(400px)より小さく
        // (しかしwrapper単体は空のページになら丸ごと収まるように)調整する。
        let filler_height = settings.content_height() - 200.0;
        let html_src = r#"<div class="filler"></div>
               <div class="wrapper">
                   <p class="a">A</p><p class="b">B</p><p class="c">C</p><p class="d">D</p>
               </div>"#;
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(&format!(
            ".filler {{ height: {filler_height}px; margin: 0; }} \
             .wrapper {{ break-inside: avoid; margin: 0; }} \
             .a, .b, .c, .d {{ height: 100px; margin: 0; }}"
        ));
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert_eq!(
            pages.len(),
            2,
            "the wrapper should move to a fresh second page instead of splitting"
        );

        let mut ps = Vec::new();
        find_all(&dom, dom.document(), "p", &mut ps);
        for &p in &ps {
            assert!(
                page_contains_content(&pages[1], p),
                "all paragraphs of the avoid-split wrapper should land on page 2"
            );
            assert!(!page_contains_content(&pages[0], p));
        }
    }

    #[test]
    fn break_inside_avoid_still_splits_when_the_element_is_taller_than_a_full_page() {
        // avoidは「できれば分割しない」という指定であり、1ページに収まらない
        // ほど巨大な場合はbest-effortで通常通り分割せざるを得ない。
        let settings = PageSettings::default();
        let mut html_src = String::from(r#"<div class="wrapper">"#);
        for i in 0..30 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(
            ".wrapper { break-inside: avoid; margin: 0; } \
             .item { height: 100px; margin: 0; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(
            pages.len() > 2,
            "a wrapper taller than a full page must still be split across pages \
             despite break-inside: avoid, got {} pages",
            pages.len()
        );
    }
}
