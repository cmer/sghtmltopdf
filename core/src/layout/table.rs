//! `display: table`要素のレイアウト(コンテンツ基準の自動列幅アルゴリズム)。
//!
//! CSS2.1 §17.5.2の自動テーブルレイアウトの簡略版。各セルの「自然な幅」を
//! (実際にはテキスト内容を折り返し無しで1行に並べた際の幅として)測り、
//! 列ごとの自然幅の最大値を求めた上で、containing widthに収まるよう
//! 比例縮尺する(containing widthの方が大きければ拡大するので、常にテーブルが
//! containing widthいっぱいに広がる。`width: auto`のテーブルがcontaining
//! blockを埋める通常のCSS挙動と一致する)。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - `rowspan`は非対応(常に1行分として扱う)
//! - `border-collapse`/`border-spacing`は非対応(セルは隙間なく直接隣接する)
//! - セル内にネストしたテーブルの自然幅測定は非対応(0として扱う)
//! - `vertical-align`は非対応(セルの内容は常に上端揃え)
//! - `<caption>`の内容はレンダリングされない([`crate::layout::box_tree`]の
//!   行収集が`table-row`のみを探すため)

use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::ComputedStyle;

use super::block::{
    box_style, layout_box_with_forced_width, resolve_border, resolve_padding, LaidOutTableRow,
};
use super::box_tree::{BoxContent, TableBox, TableCell};
use super::inline::layout_inline_content;

/// 折り返し計算を無効化するために使う、実質無限大とみなせる幅。
const UNCONSTRAINED_WIDTH: f32 = f32::MAX / 4.0;

/// テーブルをレイアウトし、レイアウト済みの行列と全体の高さを返す。
pub(super) fn layout_table(
    table: &TableBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    x: f32,
    y: f32,
) -> (Vec<LaidOutTableRow>, f32) {
    let column_count = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|cell| cell.colspan).sum::<usize>())
        .max()
        .unwrap_or(0);

    if column_count == 0 {
        return (Vec::new(), 0.0);
    }

    let col_widths = compute_column_widths(table, styles, fonts, column_count, containing_width);
    let mut col_x = vec![0.0f32; column_count + 1];
    for i in 0..column_count {
        col_x[i + 1] = col_x[i] + col_widths[i];
    }

    let mut laid_rows = Vec::with_capacity(table.rows.len());
    let mut cursor_y = y;
    for row in &table.rows {
        let mut col = 0usize;
        let mut laid_cells = Vec::with_capacity(row.cells.len());
        let mut row_height = 0.0f32;

        for cell in &row.cells {
            let span_end = (col + cell.colspan).min(column_count);
            let outer_width: f32 = col_widths[col..span_end].iter().sum();
            let cell_x = x + col_x[col];

            let cell_style = box_style(&cell.content, styles);
            let cell_padding = resolve_padding(&cell_style, outer_width);
            let cell_border = resolve_border(&cell_style);
            let content_width = (outer_width
                - cell_padding.left
                - cell_padding.right
                - cell_border.left
                - cell_border.right)
                .max(0.0);

            let laid_cell = layout_box_with_forced_width(
                &cell.content,
                styles,
                fonts,
                outer_width,
                content_width,
                cell_x,
                cursor_y,
            );
            row_height = row_height.max(laid_cell.layout.margin_box_height());
            laid_cells.push(laid_cell);
            col += cell.colspan;
        }

        // 各セルの高さを行の高さ(=最も高いセルの高さ)まで伸ばす。
        // `vertical-align`は非対応のため、伸びた分は常にcontentの下側に
        // 加える(上端揃え)。
        for cell in &mut laid_cells {
            let deficit = row_height - cell.layout.margin_box_height();
            if deficit > 0.0 {
                cell.layout.content.height += deficit;
            }
        }

        laid_rows.push(LaidOutTableRow {
            node: row.node,
            cells: laid_cells,
        });
        cursor_y += row_height;
    }

    let total_height = cursor_y - y;
    (laid_rows, total_height)
}

/// 各列の使用幅を求める。セルの内容から求めた「自然な幅」の列ごとの最大値を、
/// containing widthちょうどに収まるよう比例縮尺する。
fn compute_column_widths(
    table: &TableBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    column_count: usize,
    containing_width: f32,
) -> Vec<f32> {
    let mut natural = vec![0.0f32; column_count];

    // 1パス目: colspan=1のセルだけで各列の自然幅の最大値を求める。
    for row in &table.rows {
        let mut col = 0usize;
        for cell in &row.cells {
            if cell.colspan == 1 && col < column_count {
                natural[col] = natural[col].max(natural_cell_width(cell, styles, fonts));
            }
            col += cell.colspan;
        }
    }

    // 2パス目: colspanをまたぐセルについて、またぐ列の自然幅合計がセル自身の
    // 自然幅に満たなければ、不足分をまたぐ列へ均等に上乗せする。
    for row in &table.rows {
        let mut col = 0usize;
        for cell in &row.cells {
            if cell.colspan > 1 {
                let end = (col + cell.colspan).min(column_count);
                if end > col {
                    let span_natural_sum: f32 = natural[col..end].iter().sum();
                    let cell_natural = natural_cell_width(cell, styles, fonts);
                    if cell_natural > span_natural_sum {
                        let deficit = cell_natural - span_natural_sum;
                        let share = deficit / (end - col) as f32;
                        for w in &mut natural[col..end] {
                            *w += share;
                        }
                    }
                }
            }
            col += cell.colspan;
        }
    }

    let natural_sum: f32 = natural.iter().sum();
    if natural_sum > 0.0 {
        let scale = containing_width / natural_sum;
        natural.iter().map(|w| w * scale).collect()
    } else {
        vec![containing_width / column_count as f32; column_count]
    }
}

/// セル1つの「自然な幅」(内容を折り返し無しで並べた幅+パディング+ボーダー)。
fn natural_cell_width(
    cell: &TableCell,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
) -> f32 {
    let style = box_style(&cell.content, styles);
    // パーセンテージ指定のpaddingは、レイアウト確定前のこの時点では基準となる
    // 幅が定まらないため0を基準に解決する(簡略化)。
    let padding = resolve_padding(&style, 0.0);
    let border = resolve_border(&style);
    measure_natural_content_width(&cell.content.content, styles, fonts)
        + padding.left
        + padding.right
        + border.left
        + border.right
}

/// ボックスの内容を折り返し無しでレイアウトした場合の自然な幅を測る。
fn measure_natural_content_width(
    content: &BoxContent,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
) -> f32 {
    match content {
        BoxContent::Inline(spans) => {
            let lines = layout_inline_content(spans, styles, fonts, UNCONSTRAINED_WIDTH, 0.0, 0.0);
            lines.iter().map(|l| l.rect.width).fold(0.0f32, f32::max)
        }
        BoxContent::Blocks(children) => children
            .iter()
            .map(|child| {
                let style = box_style(child, styles);
                let padding = resolve_padding(&style, 0.0);
                let border = resolve_border(&style);
                measure_natural_content_width(&child.content, styles, fonts)
                    + padding.left
                    + padding.right
                    + border.left
                    + border.right
            })
            .fold(0.0f32, f32::max),
        // ネストしたテーブルの自然幅測定は非対応(既知の簡略化)。
        BoxContent::Table(_) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::block::{layout_document, LaidOutBox};
    use super::super::box_tree::build_box_tree;
    use super::*;
    use crate::fonts::Font;
    use crate::html::{self, Dom, NodeData};
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(TEST_FONT_PATH).expect("should load bundled test font")
        ])
    }

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    /// `Table`の中も辿る、テスト専用の`find_laid_out`。
    fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        match &b.content {
            super::super::block::LaidOutContent::Blocks(children) => children
                .iter()
                .find_map(|child| find_laid_out(child, target)),
            super::super::block::LaidOutContent::Table(rows) => rows
                .iter()
                .flat_map(|row| &row.cells)
                .find_map(|cell| find_laid_out(cell, target)),
            super::super::block::LaidOutContent::Inline(_) => None,
        }
    }

    fn layout_table_html(html_src: &str, css: &str, containing_width: f32) -> LaidOutBox {
        let dom = html::parse(html_src.as_bytes());
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(css);
        let styles = compute_styles(&dom, &ua, &author);
        let tree = build_box_tree(&dom, &styles);
        let fonts = test_fonts();
        let laid = layout_document(&tree, &styles, &fonts, containing_width);
        let table_node = find(&dom, dom.document(), "table").expect("table not found");
        find_laid_out(&laid, table_node)
            .expect("table box not found")
            .clone()
    }

    fn cell_widths(table: &LaidOutBox, row: usize) -> Vec<f32> {
        let super::super::block::LaidOutContent::Table(rows) = &table.content else {
            panic!("expected a laid-out table");
        };
        rows[row]
            .cells
            .iter()
            .map(|c| c.layout.border_box().width)
            .collect()
    }

    #[test]
    fn table_stretches_to_fill_the_containing_width() {
        // body既定のmargin(UAスタイルシート由来)を打ち消し、containing widthが
        // そのままtableに渡る状態で検証する。
        let table = layout_table_html(
            "<table><tr><td>a</td><td>bb</td></tr></table>",
            "body { margin: 0; }",
            700.0,
        );
        let super::super::block::LaidOutContent::Table(rows) = &table.content else {
            panic!("expected a laid-out table");
        };
        let total_width: f32 = rows[0]
            .cells
            .iter()
            .map(|c| c.layout.border_box().width)
            .sum();
        assert!(
            (total_width - 700.0).abs() < 0.5,
            "table should stretch to fill the containing width, got {total_width}"
        );
    }

    #[test]
    fn wider_content_gets_a_proportionally_wider_column() {
        let table = layout_table_html(
            "<table><tr><td>x</td><td>a much much much longer piece of text</td></tr></table>",
            "",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        assert!(
            widths[1] > widths[0] * 3.0,
            "the column with much longer content should be proportionally wider: {widths:?}"
        );
    }

    #[test]
    fn equal_content_produces_roughly_equal_columns() {
        // 同じ文字数でも文字が違えばグリフ幅が異なりうる(例: 'a'と'b'の
        // 送り幅は同じとは限らない)ため、自然幅が本当に同一になることを
        // 検証するには同じテキストを使う。
        let table = layout_table_html(
            "<table><tr><td>identical</td><td>identical</td></tr></table>",
            "",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        assert!(
            (widths[0] - widths[1]).abs() < 0.5,
            "identical content should produce identical column widths: {widths:?}"
        );
    }

    #[test]
    fn colspan_cell_widens_the_columns_it_spans() {
        // 3列のテーブル: 1行目は最初の2列にまたがる幅広の見出し+3列目の狭い
        // セル、2行目は3列とも同じ狭い内容("x"/"y"/"w")。列0・1は自分自身の
        // 内容(x/y)だけなら列2(w)と同じ幅になるはずだが、1行目の幅広い
        // colspanセルを賄うために列0・1が広げられ、結果として列2より
        // 明確に広くなるはず。
        let table = layout_table_html(
            r#"<table>
                <tr><td colspan="2">a much much much longer heading spanning both columns nicely</td><td>z</td></tr>
                <tr><td>x</td><td>y</td><td>w</td></tr>
            </table>"#,
            "",
            700.0,
        );
        let row1_widths = cell_widths(&table, 1);
        assert!(
            row1_widths[0] + row1_widths[1] > row1_widths[2] * 3.0,
            "columns spanned by the wide header should be widened relative to the untouched column: {row1_widths:?}"
        );
    }

    #[test]
    fn row_height_is_the_tallest_cell_in_that_row() {
        let table = layout_table_html(
            r#"<table>
                <tr><td style="height: 10px;">a</td><td style="height: 80px;">b</td></tr>
            </table>"#,
            "",
            700.0,
        );
        let super::super::block::LaidOutContent::Table(rows) = &table.content else {
            panic!("expected a laid-out table");
        };
        for cell in &rows[0].cells {
            assert_eq!(
                cell.layout.margin_box_height(),
                80.0,
                "every cell in the row should occupy the tallest cell's height"
            );
        }
    }

    #[test]
    fn cells_in_the_same_row_are_placed_side_by_side() {
        let table = layout_table_html(
            "<table><tr><td>a</td><td>b</td><td>c</td></tr></table>",
            "",
            700.0,
        );
        let super::super::block::LaidOutContent::Table(rows) = &table.content else {
            panic!("expected a laid-out table");
        };
        let cells = &rows[0].cells;
        for pair in cells.windows(2) {
            assert_eq!(
                pair[1].layout.border_box().x,
                pair[0].layout.border_box().x + pair[0].layout.border_box().width,
                "adjacent cells should touch with no gap"
            );
        }
    }

    #[test]
    fn empty_table_has_no_rows_and_zero_height() {
        let table = layout_table_html("<table></table>", "", 700.0);
        let super::super::block::LaidOutContent::Table(rows) = &table.content else {
            panic!("expected a laid-out table");
        };
        assert!(rows.is_empty());
        assert_eq!(table.layout.content.height, 0.0);
    }
}
