//! DOM+計算スタイルからレイアウトボックスツリーを構築する。
//!
//! `display: none`の要素(とその部分木)は除外する。ブロックコンテナの子が
//! block-levelとinline-level/テキストの混在になる場合は、CSSの無名ボックス生成
//! 規則(CSS2.1 9.2.1.1)に従い、連続するinline-levelの内容を無名ブロックボックスに
//! まとめる。無名ボックスは対応するDOMノードを持たないため`node: None`とする。
//!
//! インライン要素内部の行分割・実際の描画はT6の責務。ここでは、`<b>`/`<span>`
//! 等のインライン要素境界をテキストノード単位の[`InlineSpan`]として保持したまま
//! 平坦化する(要素そのものを畳み込みはするが、どの計算スタイルが適用される
//! テキストかという情報は失わない)。
//!
//! `<img>`(マイルストーン5)は`build_box_tree`/`build_box_for_element`の時点
//! では他のブロック要素と同様に組み込むだけで、実際のフェッチ・デコード
//! (I/Oを伴う)は行わない。[`resolve_images`]がbox tree構築後に別パスとして
//! 呼ばれ、`<img>`要素に対応するボックスの中身を[`BoxContent::Image`]に
//! 差し替える。構築と分離しているのは、`build_box_tree`/
//! `build_box_for_element`を「DOM+スタイルのみから決まる純粋な処理」の
//! ままに保ちたいため([0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)参照)。

use std::collections::HashMap;
use std::rc::Rc;

use crate::html::{Dom, NodeData, NodeId};
use crate::pdf::{ImageAssetCache, PreparedImage};
use crate::style::{CaptionSide, ComputedStyle, Display};

#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// 対応するDOM要素。無名ボックスの場合は`None`。
    pub node: Option<NodeId>,
    pub content: BoxContent,
}

#[derive(Debug, Clone)]
pub enum BoxContent {
    Blocks(Vec<LayoutBox>),
    /// インラインフォーマッティングコンテキストの内容。
    Inline(Vec<InlineSpan>),
    /// `display: table`要素の内容(行・セル)。
    Table(TableBox),
    /// `<img>`要素(置換要素として扱う、[`resolve_images`]参照)。
    Image(ImageBoxContent),
}

/// `<img>`要素のコンテンツ。`resolve_images`が構築する。
#[derive(Debug, Clone)]
pub struct ImageBoxContent {
    /// フェッチ・デコードに成功した場合の画像データ。失敗
    /// (ネットワークエラー・SSRFブロック・デコード不能等、いずれも同列)
    /// した場合は`None`になり、レイアウト(T53)はこれを空の置換要素として
    /// 扱う([0014]の方針)。
    pub image: Option<std::rc::Rc<crate::pdf::PreparedImage>>,
    /// `width`/`height`属性の値(px、HTML属性由来)。CSSの`width`/`height`
    /// より弱い優先度のヒントとしてレイアウト(T53)が使う。
    pub attr_width: Option<u32>,
    pub attr_height: Option<u32>,
}

/// `display: table`要素から集めた行の並びと、任意の`caption`。
#[derive(Debug, Clone)]
pub struct TableBox {
    /// `display: table-caption`の子要素(`<caption>`)。複数ある場合は最初の
    /// 1つのみ採用する(既知の簡略化、[0021](
    /// ../../../docs/decisions/0021-table-layout-design.md)決定4)。
    /// `Box`は`LayoutBox`→`BoxContent::Table`→`TableBox`の再帰を間接参照で
    /// 断ち切るために必要(サイズが無限になるコンパイルエラーの回避)。
    pub caption: Option<Box<LayoutBox>>,
    /// captionの計算スタイルから読んだ`caption-side`(captionが無ければ初期値`Top`)。
    pub caption_side: CaptionSide,
    pub rows: Vec<TableRow>,
}

/// `display: table-row`要素(`<tr>`)1行分。
#[derive(Debug, Clone)]
pub struct TableRow {
    pub node: NodeId,
    pub cells: Vec<TableCell>,
}

/// `display: table-cell`要素(`<td>`/`<th>`)1セル分。
#[derive(Debug, Clone)]
pub struct TableCell {
    pub node: NodeId,
    /// `colspan`属性の値(未指定または不正な値は1)。
    pub colspan: usize,
    /// `rowspan`属性の値(未指定または不正な値は1)。`rowspan="0"`
    /// (HTML5の「以降の行末まで拡張」特殊値)は非対応、1として扱う
    /// ([0021](../../../docs/decisions/0021-table-layout-design.md)既知の簡略化4)。
    pub rowspan: usize,
    /// セル自身の内容(通常のブロック/インラインボックスと同じ構造)。
    pub content: LayoutBox,
}

/// 1つのDOMテキストノードに由来する、単一の計算スタイルを持つテキスト区間。
#[derive(Debug, Clone)]
pub struct InlineSpan {
    /// このテキストの元になったDOMテキストノード。`styles`から計算スタイルを
    /// 引く(`<b>`/`<span style="...">`等の祖先の宣言は、テキストノード自身の
    /// 計算スタイルに継承・カスケード済みなので、このノードのスタイルを見れば足りる)。
    pub node: NodeId,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildKind {
    /// `display: none`、空白のみのテキストなど、ボックスを生成しない。
    None,
    Block,
    Inline,
}

pub fn build_box_tree(dom: &Dom, styles: &HashMap<NodeId, ComputedStyle>) -> LayoutBox {
    let child_ids: Vec<NodeId> = dom.children(dom.document()).collect();
    LayoutBox {
        node: None,
        content: BoxContent::Blocks(build_children_boxes(dom, styles, &child_ids)),
    }
}

/// `node`単体(とその子孫)から[`LayoutBox`]を構築する。`build_box_tree`が
/// 文書全体を辿る際の内部処理だが、マイルストーン3のストリーミング処理では
/// 「切り出したトップレベル要素1つ分だけ」の`LayoutBox`を作るために直接使う
/// (`build_box_tree`のように`dom.document()`の子全部を辿るのではなく、
/// 特定の`node`だけを対象にする)。
pub(crate) fn build_box_for_element(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
) -> Option<LayoutBox> {
    let style = styles.get(&node)?;
    if style.display == Display::None {
        return None;
    }
    if style.display == Display::Table {
        return Some(LayoutBox {
            node: Some(node),
            content: BoxContent::Table(build_table_box(dom, styles, node)),
        });
    }

    let child_ids: Vec<NodeId> = dom.children(node).collect();
    let has_block_child = child_ids
        .iter()
        .any(|&c| child_kind(dom, styles, c) == ChildKind::Block);

    let content = if has_block_child {
        // `::before`/`::after`はブロック子を持つ要素では非対応(簡略化)。
        // 無名ボックス生成規則との組み合わせが複雑になるため見送る。
        BoxContent::Blocks(build_children_boxes(dom, styles, &child_ids))
    } else {
        let mut spans = Vec::new();
        push_before_content(styles, node, &mut spans);
        for &child in &child_ids {
            if child_kind(dom, styles, child) == ChildKind::Inline {
                collect_spans(dom, styles, child, &mut spans);
            }
        }
        push_after_content(styles, node, &mut spans);
        BoxContent::Inline(spans)
    };

    Some(LayoutBox {
        node: Some(node),
        content,
    })
}

/// box tree構築後に呼び、`<img>`要素に対応するボックス(`child_kind`により
/// ブロック扱いされ、この時点では中身が空の`BoxContent::Inline(vec![])`に
/// なっている)を実際に[`BoxContent::Image`]へ差し替える。
///
/// `image_cache`がフェッチ・デコードを行う(I/Oを伴う)。同じ`src`は
/// `image_cache`内でメモ化されるため、同一画像が繰り返し参照されても
/// 実際のフェッチ・デコードは初回のみ([0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)参照)。
pub fn resolve_images(tree: &mut LayoutBox, dom: &Dom, image_cache: &ImageAssetCache) {
    if let Some(node) = tree.node {
        if let NodeData::Element { name, .. } = &dom.node(node).data {
            if &*name.local == "img" {
                tree.content = BoxContent::Image(build_image_box_content(dom, node, image_cache));
                return; // <img>はvoid element(子を持たない)なので再帰不要。
            }
        }
    }

    match &mut tree.content {
        BoxContent::Blocks(children) => {
            for child in children {
                resolve_images(child, dom, image_cache);
            }
        }
        BoxContent::Table(table) => {
            if let Some(caption) = &mut table.caption {
                resolve_images(caption, dom, image_cache);
            }
            for row in &mut table.rows {
                for cell in &mut row.cells {
                    resolve_images(&mut cell.content, dom, image_cache);
                }
            }
        }
        BoxContent::Inline(_) | BoxContent::Image(_) => {}
    }
}

/// `background-image`が指定された要素の、デコード済み画像を`NodeId`キーで
/// 引けるようにする側マップを構築する。[0017](../../../docs/decisions/0017-background-image-design.md)
/// 決定2により、`<img>`の[`resolve_images`]と異なりbox tree(`LayoutBox`)の
/// 中身は一切変更しない(背景画像はレイアウトのサイズ計算に影響しない、
/// 描画専用の情報のため)。DOM木の再走査も不要で、カスケード計算済みの
/// `styles`を`background_image.is_some()`でフィルタするだけで済む。
///
/// フェッチ・デコードに失敗した要素は、その要素だけ背景画像なし扱いにして
/// マップに含めない(0014と同じフォールバック方針、文書全体は止めない)。
pub fn resolve_background_images(
    styles: &HashMap<NodeId, ComputedStyle>,
    image_cache: &ImageAssetCache,
) -> HashMap<NodeId, Rc<PreparedImage>> {
    let mut out = HashMap::new();
    for (&node, style) in styles {
        let Some(url) = &style.background_image else {
            continue;
        };
        if let Ok(image) = image_cache.get_or_decode(url) {
            out.insert(node, image);
        }
    }
    out
}

fn build_image_box_content(
    dom: &Dom,
    node: NodeId,
    image_cache: &ImageAssetCache,
) -> ImageBoxContent {
    let attrs = crate::img::read_img_attrs(dom, node);
    let image = attrs
        .as_ref()
        .and_then(|a| image_cache.get_or_decode(&a.src).ok());
    ImageBoxContent {
        image,
        attr_width: attrs.as_ref().and_then(|a| a.width),
        attr_height: attrs.as_ref().and_then(|a| a.height),
    }
}

fn build_children_boxes(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    child_ids: &[NodeId],
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    let mut pending_spans: Vec<InlineSpan> = Vec::new();

    for &child in child_ids {
        match child_kind(dom, styles, child) {
            ChildKind::None => {}
            ChildKind::Block => {
                flush_pending_spans(&mut pending_spans, &mut result);
                if let Some(b) = build_box_for_element(dom, styles, child) {
                    result.push(b);
                }
            }
            ChildKind::Inline => collect_spans(dom, styles, child, &mut pending_spans),
        }
    }
    flush_pending_spans(&mut pending_spans, &mut result);

    result
}

/// `table_node`(`display: table`)の子孫から`table-row`要素と`caption`を集めて
/// [`TableBox`]を組み立てる。
fn build_table_box(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    table_node: NodeId,
) -> TableBox {
    let mut rows = Vec::new();
    let mut caption_node = None;
    collect_table_rows(dom, styles, table_node, &mut rows, &mut caption_node);

    let caption_side = caption_node
        .and_then(|node| styles.get(&node))
        .map(|s| s.caption_side)
        .unwrap_or_default();
    let caption = caption_node
        .and_then(|node| build_box_for_element(dom, styles, node))
        .map(Box::new);

    TableBox {
        caption,
        caption_side,
        rows,
    }
}

/// `node`の子を辿り、`table-row`を見つけたら行として収集し、`table-caption`を
/// 見つけたら(最初の1つだけ)`out_caption`に記録する。`thead`/`tbody`/`tfoot`
/// のような透過的な入れ物(`table-row`/`table-caption`でも`table`でもない要素)は
/// 素通りして再帰する。入れ子の`table`はそれ自体が別のテーブルなので
/// (その中の行は内側のテーブルに属する)ここでは再帰しない。
fn collect_table_rows(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
    out: &mut Vec<TableRow>,
    out_caption: &mut Option<NodeId>,
) {
    for child in dom.children(node) {
        match styles.get(&child).map(|s| s.display) {
            Some(Display::TableRow) => out.push(build_table_row(dom, styles, child)),
            Some(Display::TableCaption) => {
                if out_caption.is_none() {
                    *out_caption = Some(child);
                }
            }
            Some(Display::Table) | Some(Display::None) | None => {}
            _ => collect_table_rows(dom, styles, child, out, out_caption),
        }
    }
}

fn build_table_row(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    row_node: NodeId,
) -> TableRow {
    let cells = dom
        .children(row_node)
        .filter(|&child| styles.get(&child).map(|s| s.display) == Some(Display::TableCell))
        .map(|cell_node| TableCell {
            node: cell_node,
            colspan: read_colspan(dom, cell_node),
            rowspan: read_rowspan(dom, cell_node),
            content: build_box_for_element(dom, styles, cell_node).unwrap_or(LayoutBox {
                node: Some(cell_node),
                content: BoxContent::Inline(Vec::new()),
            }),
        })
        .collect();
    TableRow {
        node: row_node,
        cells,
    }
}

/// `colspan`属性を読む(未指定・0以下・非数値は1として扱う)。
fn read_colspan(dom: &Dom, node: NodeId) -> usize {
    let NodeData::Element { attrs, .. } = &dom.node(node).data else {
        return 1;
    };
    attrs
        .iter()
        .find(|attr| &*attr.name.local == "colspan")
        .and_then(|attr| attr.value.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

/// `rowspan`属性を読む(未指定・0以下・非数値は1として扱う。`rowspan="0"`の
/// 特殊値も非対応で1扱い、`read_colspan`と同じ方針)。
fn read_rowspan(dom: &Dom, node: NodeId) -> usize {
    let NodeData::Element { attrs, .. } = &dom.node(node).data else {
        return 1;
    };
    attrs
        .iter()
        .find(|attr| &*attr.name.local == "rowspan")
        .and_then(|attr| attr.value.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

fn flush_pending_spans(pending: &mut Vec<InlineSpan>, result: &mut Vec<LayoutBox>) {
    if !pending.iter().all(|span| span.text.trim().is_empty()) {
        result.push(LayoutBox {
            node: None,
            content: BoxContent::Inline(std::mem::take(pending)),
        });
    }
    pending.clear();
}

fn child_kind(dom: &Dom, styles: &HashMap<NodeId, ComputedStyle>, node: NodeId) -> ChildKind {
    match &dom.node(node).data {
        NodeData::Element { name, .. } => {
            let display = styles.get(&node).map(|s| s.display);
            if display == Some(Display::None) {
                return ChildKind::None;
            }
            // `<img>`はHTML上のdisplay既定値がinlineの置換要素だが、インライン
            // フォーマッティングコンテキスト(行分割・ベースライン整合)への
            // 統合は複雑になるため、M5の初期スコープではブロックレベルの
            // 置換要素としてのみ扱う([[0014]]参照、既知の制約)。
            if &*name.local == "img" {
                return ChildKind::Block;
            }
            match display {
                Some(Display::Block) | Some(Display::Table) => ChildKind::Block,
                Some(Display::Inline) => ChildKind::Inline,
                // table-row/table-cell/table-captionは`build_table_box`が専用に
                // 探索するため、通常のブロック/インライン走査では(不正な
                // マークアップ等でテーブル文脈の外に出現しない限り)出現しない。
                // 防御的に無視する。
                Some(Display::TableRow)
                | Some(Display::TableCell)
                | Some(Display::TableCaption) => ChildKind::None,
                Some(Display::None) | None => ChildKind::None,
            }
        }
        NodeData::Text { contents } => {
            if contents.trim().is_empty() {
                ChildKind::None
            } else {
                ChildKind::Inline
            }
        }
        _ => ChildKind::None,
    }
}

/// インライン要素の子孫を再帰的に辿り、テキストノードごとに[`InlineSpan`]を積む。
/// テキストノード自身の計算スタイルに、祖先のインライン要素(`<b>`/`<span>`等)の
/// カスケード・継承結果が反映済みのため、ここではノードIDを保持するだけでよい。
/// 各インライン要素の`::before`/`::after`生成コンテンツも、対応する子孫の
/// 前後にスパンとして挿入する。
fn collect_spans(
    dom: &Dom,
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
    out: &mut Vec<InlineSpan>,
) {
    match &dom.node(node).data {
        NodeData::Text { contents } => out.push(InlineSpan {
            node,
            text: contents.clone(),
        }),
        NodeData::Element { .. } => {
            push_before_content(styles, node, out);
            for child in dom.children(node) {
                collect_spans(dom, styles, child, out);
            }
            push_after_content(styles, node, out);
        }
        _ => {}
    }
}

/// `node`に`::before`の生成コンテンツがあれば、その計算スタイルを引くための
/// ノードID(`node`自身)と共にスパンを積む。
fn push_before_content(
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
    out: &mut Vec<InlineSpan>,
) {
    if let Some(text) = styles
        .get(&node)
        .and_then(|s| s.pseudo_before_content.as_ref())
    {
        out.push(InlineSpan {
            node,
            text: text.clone(),
        });
    }
}

/// `node`に`::after`の生成コンテンツがあれば、その計算スタイルを引くための
/// ノードID(`node`自身)と共にスパンを積む。
fn push_after_content(
    styles: &HashMap<NodeId, ComputedStyle>,
    node: NodeId,
    out: &mut Vec<InlineSpan>,
) {
    if let Some(text) = styles
        .get(&node)
        .and_then(|s| s.pseudo_after_content.as_ref())
    {
        out.push(InlineSpan {
            node,
            text: text.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    use crate::style::{
        compute_styles, parse_stylesheet, user_agent_stylesheet, RgbaColor, Stylesheet,
    };

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    fn find_inline_spans(b: &LayoutBox) -> Option<&Vec<InlineSpan>> {
        match &b.content {
            BoxContent::Inline(spans) => Some(spans),
            BoxContent::Blocks(children) => children.iter().find_map(find_inline_spans),
            BoxContent::Image(_) => None,
            BoxContent::Table(table) => table
                .caption
                .as_deref()
                .and_then(find_inline_spans)
                .or_else(|| {
                    table
                        .rows
                        .iter()
                        .flat_map(|row| &row.cells)
                        .find_map(|cell| find_inline_spans(&cell.content))
                }),
        }
    }

    #[test]
    fn inline_element_boundaries_are_preserved_as_separate_spans() {
        let dom = html::parse(br#"<p>before <b>bold</b> after</p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let p = find(&dom, dom.document(), "p").expect("p not found");
        let b = find(&dom, p, "b").expect("b not found");
        let p_box = find_box(&tree, p).expect("p box not found");
        let spans = find_inline_spans(p_box).expect("expected inline content");

        assert_eq!(spans.len(), 3, "before-text / bold-text / after-text");
        assert_eq!(spans[0].text, "before ");
        assert_eq!(spans[1].text, "bold");
        assert_eq!(spans[2].text, " after");
        // 太字テキストのスパンは<b>の子テキストノード由来であり、<p>直下のテキストとは
        // 別のNodeIdを持つ(=別の計算スタイルを引ける)。
        assert_ne!(spans[0].node, spans[1].node);
        assert_eq!(dom.children(b).next(), Some(spans[1].node));
    }

    #[test]
    fn span_style_reflects_ancestor_cascade_at_layout_time() {
        let dom = html::parse(br#"<p>plain <b style="color: rgb(9, 9, 9);">loud</b></p>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let p = find(&dom, dom.document(), "p").expect("p not found");
        let p_box = find_box(&tree, p).expect("p box not found");
        let spans = find_inline_spans(p_box).expect("expected inline content");

        let loud_style = &styles[&spans[1].node];
        assert_eq!(
            loud_style.color,
            RgbaColor {
                red: 9,
                green: 9,
                blue: 9,
                alpha: 1.0
            }
        );
        assert_eq!(loud_style.font_weight, crate::style::FontWeight::Bold);
    }

    #[test]
    fn before_and_after_content_are_prepended_and_appended_as_spans() {
        // <span>はインライン要素なので、単独では自分自身のLayoutBoxを持たず
        // 祖先のブロックコンテナ(ここでは<body>)の平坦化されたスパン列に
        // 織り込まれる。それでも::before/::afterは正しく前後に挿入されるはず。
        let dom = html::parse(br#"<span class="badge">Text</span>"#);
        let ua = user_agent_stylesheet();
        let author = crate::style::parse_stylesheet(
            r#".badge::before { content: "["; } .badge::after { content: "]"; }"#,
        );
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let span = find(&dom, dom.document(), "span").expect("span not found");
        let spans = find_inline_spans(&tree).expect("expected inline content");

        assert_eq!(spans.len(), 3, "before / text / after");
        assert_eq!(spans[0].text, "[");
        assert_eq!(spans[1].text, "Text");
        assert_eq!(spans[2].text, "]");
        // 生成コンテンツのスパンはホスト要素自身のノードIDを持つ
        // (=ホストの計算スタイルをそのまま流用する)。
        assert_eq!(spans[0].node, span);
        assert_eq!(spans[2].node, span);
    }

    #[test]
    fn element_without_before_after_rules_has_no_extra_spans() {
        let dom = html::parse(br#"<span>Text</span>"#);
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);

        let spans = find_inline_spans(&tree).expect("expected inline content");

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "Text");
    }

    #[test]
    fn table_rows_and_cells_are_collected_through_thead_tbody() {
        let dom = html::parse(
            br#"<table>
                <thead><tr><th>Name</th><th>Price</th></tr></thead>
                <tbody>
                    <tr><td>Apple</td><td>100</td></tr>
                    <tr><td>Banana</td><td>200</td></tr>
                </tbody>
            </table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let table_box = find_box(&tree, table_node).expect("table box not found");
        let BoxContent::Table(table) = &table_box.content else {
            panic!("expected a table box");
        };

        assert_eq!(table.rows.len(), 3, "thead + 2 tbody rows");
        assert_eq!(table.rows[0].cells.len(), 2);
        let first_cell_text = |content: &LayoutBox| match &content.content {
            BoxContent::Inline(spans) => spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            _ => panic!("expected inline cell content"),
        };
        assert_eq!(first_cell_text(&table.rows[0].cells[0].content), "Name");
        assert_eq!(first_cell_text(&table.rows[1].cells[0].content), "Apple");
        assert_eq!(first_cell_text(&table.rows[2].cells[0].content), "Banana");
    }

    #[test]
    fn caption_content_is_collected_and_kept_separate_from_rows() {
        let dom = html::parse(
            br#"<table>
                <caption>Fruit Prices</caption>
                <tr><td>Apple</td><td>100</td></tr>
            </table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let table_box = find_box(&tree, table_node).expect("table box not found");
        let BoxContent::Table(table) = &table_box.content else {
            panic!("expected a table box");
        };

        assert_eq!(
            table.rows.len(),
            1,
            "caption should not be collected as a row"
        );
        let caption = table.caption.as_ref().expect("caption not found");
        let BoxContent::Inline(spans) = &caption.content else {
            panic!("expected inline caption content");
        };
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "Fruit Prices");
    }

    #[test]
    fn table_without_a_caption_has_none() {
        let dom = html::parse(br#"<table><tr><td>a</td></tr></table>"#);
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let table_box = find_box(&tree, table_node).expect("table box not found");
        let BoxContent::Table(table) = &table_box.content else {
            panic!("expected a table box");
        };
        assert!(table.caption.is_none());
    }

    #[test]
    fn colspan_attribute_is_read_from_the_cell() {
        let dom =
            html::parse(br#"<table><tr><td colspan="3">wide</td><td>narrow</td></tr></table>"#);
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let BoxContent::Table(table) = &find_box(&tree, table_node).unwrap().content else {
            panic!("expected a table box");
        };
        assert_eq!(table.rows[0].cells[0].colspan, 3);
        assert_eq!(table.rows[0].cells[1].colspan, 1);
    }

    #[test]
    fn invalid_or_missing_colspan_defaults_to_one() {
        let dom = html::parse(
            br#"<table><tr><td colspan="0">a</td><td colspan="not-a-number">b</td><td>c</td></tr></table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let BoxContent::Table(table) = &find_box(&tree, table_node).unwrap().content else {
            panic!("expected a table box");
        };
        for cell in &table.rows[0].cells {
            assert_eq!(cell.colspan, 1);
        }
    }

    #[test]
    fn rowspan_attribute_is_read_from_the_cell() {
        let dom = html::parse(
            br#"<table>
                <tr><td rowspan="2">tall</td><td>a</td></tr>
                <tr><td>b</td></tr>
            </table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let BoxContent::Table(table) = &find_box(&tree, table_node).unwrap().content else {
            panic!("expected a table box");
        };
        assert_eq!(table.rows[0].cells[0].rowspan, 2);
        assert_eq!(table.rows[0].cells[1].rowspan, 1);
        assert_eq!(table.rows[1].cells[0].rowspan, 1);
    }

    #[test]
    fn invalid_or_missing_rowspan_defaults_to_one() {
        let dom = html::parse(
            br#"<table><tr><td rowspan="0">a</td><td rowspan="not-a-number">b</td><td>c</td></tr></table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        let BoxContent::Table(table) = &find_box(&tree, table_node).unwrap().content else {
            panic!("expected a table box");
        };
        for cell in &table.rows[0].cells {
            assert_eq!(cell.rowspan, 1);
        }
    }

    #[test]
    fn nested_table_rows_belong_to_the_inner_table_only() {
        // 入れ子のtableの<tr>は、内側のtableに属し、外側のtableの行としては
        // 収集されないはず。
        let dom = html::parse(
            br#"<table id="outer"><tr><td>
                <table id="inner"><tr><td>nested</td></tr></table>
            </td></tr></table>"#,
        );
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let tree = build_box_tree(&dom, &styles);

        let outer_node = find(&dom, dom.document(), "table").expect("outer table not found");
        let BoxContent::Table(outer_table) = &find_box(&tree, outer_node).unwrap().content else {
            panic!("expected a table box");
        };

        assert_eq!(
            outer_table.rows.len(),
            1,
            "outer table should have exactly one row"
        );
        assert_eq!(outer_table.rows[0].cells.len(), 1);
        // 外側の唯一のセルの中身はブロックコンテナ(内側のtableを含む)であり、
        // 内側のtableの行が紛れ込んでいないはず。
        let BoxContent::Blocks(cell_children) = &outer_table.rows[0].cells[0].content.content
        else {
            panic!("expected the outer cell to contain a block (the nested table)")
        };
        assert_eq!(cell_children.len(), 1);
        let BoxContent::Table(inner_table) = &cell_children[0].content else {
            panic!("expected the nested table box")
        };
        assert_eq!(inner_table.rows.len(), 1);
    }

    fn find_box(b: &LayoutBox, target: NodeId) -> Option<&LayoutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        if let BoxContent::Blocks(children) = &b.content {
            for child in children {
                if let Some(found) = find_box(child, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn jpeg_data_uri() -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/images/spike_gradient.jpg"
        );
        let bytes = std::fs::read(path).unwrap();
        format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
    }

    #[test]
    fn resolve_background_images_decodes_only_nodes_with_background_image_set() {
        // [0017]決定2: `resolve_background_images`はDOM木の再走査をせず、
        // カスケード計算済みの`styles`を`background_image.is_some()`で
        // フィルタするだけで側マップを構築できるはず。
        let dom = html::parse(br#"<div><p>text</p></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");
        let p = find(&dom, div, "p").expect("p not found");

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(&format!(
            r#"div {{ background-image: url("{}"); }}"#,
            jpeg_data_uri()
        ));
        let styles = compute_styles(&dom, &ua, &author);

        let image_cache = ImageAssetCache::new(std::path::PathBuf::from("."), false);
        let background_images = resolve_background_images(&styles, &image_cache);

        assert!(
            background_images.contains_key(&div),
            "div should have a decoded background image"
        );
        assert!(
            !background_images.contains_key(&p),
            "p has no background-image declared and should not be in the map"
        );
    }

    #[test]
    fn resolve_background_images_skips_a_failed_fetch_without_panicking() {
        let dom = html::parse(br#"<div></div>"#);
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(r#"div { background-image: url("does-not-exist.png"); }"#);
        let styles = compute_styles(&dom, &ua, &author);

        let image_cache = ImageAssetCache::new(std::path::PathBuf::from("."), false);
        let background_images = resolve_background_images(&styles, &image_cache);

        assert!(
            background_images.is_empty(),
            "a failed background-image fetch should be skipped, not panic"
        );
    }
}
