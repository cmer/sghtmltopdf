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
//! - `rowspan="0"`(HTML5の「以降の行末まで拡張」特殊値)は非対応、1として扱う
//! - `border-collapse: collapse`は見た目の枠線描画のみ統合し、レイアウト計算は
//!   separateモデルと同一
//! - セル内にネストしたテーブルの自然幅測定は非対応(0として扱う)
//! - `vertical-align: baseline`でベースラインを提供できないセル内容
//!   (ネストしたテーブル・置換要素)は`bottom`相当にフォールバックする

use std::collections::HashMap;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::{
    CaptionSide, ComputedStyle, LengthPercentageOrAuto, TableLayout, VerticalAlign,
};

use super::block::{
    box_style, clamp_used_width, layout_box_ignoring_positioned,
    layout_box_with_forced_width_ignoring_positioned, resolve_border, resolve_lp, resolve_padding,
    shift_box_y, shift_content_vertical, LaidOutBox, LaidOutContent, LaidOutTable, LaidOutTableRow,
};
use super::box_tree::{BoxContent, TableBox, TableCell, TableRow};
use super::float_ctx::FloatContext;
use super::inline::layout_inline_content;

/// 折り返し計算を無効化するために使う、実質無限大とみなせる幅。
const UNCONSTRAINED_WIDTH: f32 = f32::MAX / 4.0;

/// テーブルをレイアウトし、レイアウト済みのテーブル(caption+行列)と全体の
/// 高さを返す。`table_layout`は`display: table`要素自身の`table-layout`計算値
/// (非継承プロパティ、呼び出し元がテーブル要素自身のスタイルから読んで渡す)。
/// `h_spacing`/`v_spacing`は`border-spacing`の解決済み値(呼び出し元が
/// `border-collapse: collapse`の場合は0に潰して渡す)。
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_table(
    table: &TableBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    containing_width: f32,
    table_layout: TableLayout,
    h_spacing: f32,
    v_spacing: f32,
    x: f32,
    y: f32,
) -> (LaidOutTable, f32) {
    // captionは行が無くても独立してレイアウトする(空テーブル+captionのみの
    // ケースにも対応するため、column_count==0の早期リターンより前に行う)。
    // captionも新しいBlock Formatting Contextを確立するとみなし、外側の
    // floatとは独立させる(テーブル本体のセルと同じ方針)。
    let laid_caption = table.caption.as_deref().map(|caption| {
        let mut caption_float_ctx = FloatContext::new();
        layout_box_ignoring_positioned(
            caption,
            styles,
            fonts,
            containing_width,
            &mut caption_float_ctx,
            x,
            y,
        )
    });
    let caption_height = laid_caption
        .as_ref()
        .map(|c| c.layout.margin_box_height())
        .unwrap_or(0.0);
    let caption_is_top = table.caption_side == CaptionSide::Top;
    // `caption-side: top`ならcaptionの高さ分だけ行の開始位置を下にずらす。
    // `bottom`なら行は`y`からそのまま開始し、captionは行レイアウト後に
    // その下へシフトする(下記)。`rows_block_start`は行群全体(前後の
    // `v_spacing`込み)の開始位置、`rows_start_y`は実際に1行目が置かれる位置。
    let rows_block_start = if caption_is_top {
        y + caption_height
    } else {
        y
    };
    let rows_start_y = rows_block_start + v_spacing;

    let column_count = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|cell| cell.colspan).sum::<usize>())
        .max()
        .unwrap_or(0);

    if column_count == 0 {
        let (caption, total_height) = match laid_caption {
            Some(c) => (Some(Box::new(c)), caption_height),
            None => (None, 0.0),
        };
        return (
            LaidOutTable {
                caption,
                caption_side: table.caption_side,
                rows: Vec::new(),
            },
            total_height,
        );
    }

    // `border-spacing`(separateモデル)は列間だけでなくテーブル外枠と最外列の
    // 間にも入る(CSS2.1 17.6.1)ため、列幅の計算に使える幅は`(列数+1)`個分の
    // `h_spacing`を差し引いたもの。
    let available_column_width =
        (containing_width - h_spacing * (column_count + 1) as f32).max(0.0);

    // rowspan/colspanのoccupancyを考慮したグリッド配置。以降の列幅・行の
    // 高さ・セル配置は全てこのグリッド経由で計算する。rowspanが全て1の場合、
    // `col += cell.colspan`による単純な列
    // カーソル走査と完全に同一の結果になる。
    let grid = build_table_grid(&table.rows, column_count);

    // `<colgroup>`/`<col>`由来の列幅ヒントを、この時点で使用幅(px)へ
    // 解決する。列数を超える分は捨て、足りない分は指定なしとして扱う。
    let column_hints: Vec<Option<f32>> = (0..column_count)
        .map(|i| {
            table
                .column_widths
                .get(i)
                .copied()
                .flatten()
                .map(|lp| resolve_lp(lp, available_column_width))
        })
        .collect();

    // `table-layout: fixed`は`<col>`と最初の行の明示`width`指定のみを見て、
    // 内容測定(`compute_column_widths`のセル自然幅計算)を完全にスキップする
    // 高速パス(仕様上もこれがfixedモードの目的そのもの)。
    let col_widths = if table_layout == TableLayout::Fixed {
        compute_fixed_column_widths(
            &grid,
            styles,
            &column_hints,
            column_count,
            available_column_width,
        )
    } else {
        compute_column_widths(
            &grid,
            styles,
            fonts,
            &column_hints,
            column_count,
            available_column_width,
        )
    };
    let mut col_x = vec![0.0f32; column_count + 1];
    col_x[0] = h_spacing;
    for i in 0..column_count {
        col_x[i + 1] = col_x[i] + col_widths[i] + h_spacing;
    }

    // 1パス目: 各セルをy=0の仮位置でレイアウトし、行の高さ確定前の「自然な
    // 高さ」を求める(rowspanで複数行にまたがるセルの実際の位置は、後段で
    // 行の高さが全て確定してから`shift_box_y`でまとめて動かす)。
    let laid_grid: Vec<Vec<LaidOutBox>> = grid
        .iter()
        .map(|row_cells| {
            row_cells
                .iter()
                .map(|gc| {
                    let outer_width: f32 = if gc.col_end > gc.col_start {
                        col_x[gc.col_end] - col_x[gc.col_start] - h_spacing
                    } else {
                        0.0
                    };
                    let cell_x = x + col_x[gc.col_start];

                    let cell_style = box_style(&gc.cell.content, styles);
                    let cell_padding = resolve_padding(&cell_style, outer_width);
                    let cell_border = resolve_border(&cell_style);
                    let content_width = (outer_width
                        - cell_padding.left
                        - cell_padding.right
                        - cell_border.left
                        - cell_border.right)
                        .max(0.0);

                    // `display: table`のセルは新しいBlock Formatting Contextを
                    // 確立する(CSS2.1 9.4.1)ため、外側のfloatとは独立した空の
                    // コンテキストを渡す。
                    let mut cell_float_ctx = FloatContext::new();
                    layout_box_with_forced_width_ignoring_positioned(
                        &gc.cell.content,
                        styles,
                        fonts,
                        outer_width,
                        content_width,
                        &mut cell_float_ctx,
                        cell_x,
                        0.0,
                    )
                })
                .collect()
        })
        .collect();

    let row_count = table.rows.len();
    // 2パス目: rowspan=1のセルだけで各行の自然な高さの最大値を求める
    // (colspanの列幅計算と同じ2パス方式)。
    let mut row_natural = vec![0.0f32; row_count];
    for (row_cells, laid_row) in grid.iter().zip(laid_grid.iter()) {
        for (gc, laid_cell) in row_cells.iter().zip(laid_row.iter()) {
            if gc.cell.rowspan == 1 {
                row_natural[gc.row_index] =
                    row_natural[gc.row_index].max(laid_cell.layout.margin_box_height());
            }
        }
    }
    // 3パス目: rowspanで複数行にまたがるセルについて、またぐ行の自然な高さの
    // 合計(+行間の`v_spacing`、colspanのh_spacingと同じ理屈)がセル自身の
    // 自然な高さに満たなければ、不足分をまたぐ行へ均等に上乗せする。
    for (row_cells, laid_row) in grid.iter().zip(laid_grid.iter()) {
        for (gc, laid_cell) in row_cells.iter().zip(laid_row.iter()) {
            if gc.cell.rowspan > 1 {
                let end = (gc.row_index + gc.cell.rowspan).min(row_count);
                if end > gc.row_index {
                    let span_count = end - gc.row_index;
                    let span_natural_sum: f32 = row_natural[gc.row_index..end].iter().sum::<f32>()
                        + v_spacing * (span_count - 1) as f32;
                    let cell_natural = laid_cell.layout.margin_box_height();
                    if cell_natural > span_natural_sum {
                        let deficit = cell_natural - span_natural_sum;
                        let share = deficit / span_count as f32;
                        for h in &mut row_natural[gc.row_index..end] {
                            *h += share;
                        }
                    }
                }
            }
        }
    }

    // 4パス目: 各行の絶対Y位置(行群の先頭からの累積、前後の`v_spacing`込み)を求める。
    let mut row_y = vec![0.0f32; row_count + 1];
    row_y[0] = rows_start_y;
    for r in 0..row_count {
        row_y[r + 1] = row_y[r] + row_natural[r] + v_spacing;
    }

    // 5パス目: 各セルを仮位置(y=0)から本来の行位置へ平行移動し、
    // rowspanで確定した最終高さまで伸ばす。伸びた分の余白の配り方は
    // `vertical-align`の計算値で決まる(top/middle/bottom/baselineの4分岐、
    // CSS2.1 17.5.3)。
    let mut laid_rows = Vec::with_capacity(table.rows.len());
    for (row, (row_cells, laid_row)) in table.rows.iter().zip(grid.iter().zip(laid_grid)) {
        let mut laid_cells: Vec<LaidOutBox> = row_cells
            .iter()
            .zip(laid_row)
            .map(|(gc, laid_cell)| shift_box_y(&laid_cell, -row_y[gc.row_index]))
            .collect();

        // ベースライン揃えのセルについて、自身の先頭行のベースラインがセル
        // 上端からどれだけ下かを求め、行全体で最大のものを「行のベースライン
        // 位置」として揃える基準にする(その行で始まるセルのみが対象。
        // CSS2.1の定義通り)。
        let row_baseline_offset = laid_cells
            .iter()
            .zip(row_cells.iter())
            .filter(|(cell, _)| cell_vertical_align(cell, styles) == VerticalAlign::Baseline)
            .filter_map(|(cell, _)| own_baseline_offset(cell, fonts))
            .fold(0.0f32, f32::max);

        for (cell, gc) in laid_cells.iter_mut().zip(row_cells.iter()) {
            let span_end = (gc.row_index + gc.cell.rowspan).min(row_count);
            let final_height = if span_end > gc.row_index {
                row_y[span_end] - row_y[gc.row_index] - v_spacing
            } else {
                0.0
            };
            let deficit = final_height - cell.layout.margin_box_height();
            if deficit <= 0.0 {
                continue;
            }
            cell.layout.content.height += deficit;

            let shift_down = match cell_vertical_align(cell, styles) {
                VerticalAlign::Top => 0.0,
                VerticalAlign::Middle => deficit / 2.0,
                VerticalAlign::Bottom => deficit,
                // `sub`/`super`/`text-top`/`text-bottom`/長さはインライン文脈
                // 専用の値で、CSS2.1ではテーブルセルに適用できない。`baseline`
                // として扱う。
                VerticalAlign::Baseline
                | VerticalAlign::Sub
                | VerticalAlign::Super
                | VerticalAlign::TextTop
                | VerticalAlign::TextBottom
                | VerticalAlign::LengthPercentage(_) => match own_baseline_offset(cell, fonts) {
                    Some(own_offset) => row_baseline_offset - own_offset,
                    // ベースラインを提供できないセル内容は`bottom`相当に
                    // フォールバックする(既知の簡略化、ファイル冒頭のコメント参照)。
                    None => deficit,
                },
            };
            if shift_down > 0.0 {
                *cell = shift_content_vertical(cell, -shift_down);
            }
        }

        laid_rows.push(LaidOutTableRow {
            node: row.node,
            cells: laid_cells,
            section: row.section,
        });
    }

    let rows_height = row_y[row_count] - rows_block_start;
    let total_height = caption_height + rows_height;

    let final_caption = match (laid_caption, caption_is_top) {
        (Some(c), true) => Some(Box::new(c)),
        (Some(c), false) => {
            // captionは`y`地点でレイアウト済みなので、行の下(`rows_height`
            // 分下)へシフトする(`shift_box_y`のdeltaは「引く」方向なので
            // 負の値を渡す)。
            Some(Box::new(shift_box_y(&c, -rows_height)))
        }
        (None, _) => None,
    };

    (
        LaidOutTable {
            caption: final_caption,
            caption_side: table.caption_side,
            rows: laid_rows,
        },
        total_height,
    )
}

/// セル(`display: table-cell`)自身の`vertical-align`計算値。
fn cell_vertical_align(
    cell: &LaidOutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
) -> VerticalAlign {
    cell.node
        .and_then(|n| styles.get(&n))
        .map(|s| s.vertical_align)
        .unwrap_or_default()
}

/// セル内容の最初の行のベースラインが、セル自身の上端
/// (`cell.layout.content.y`)からどれだけ下にあるかを求める。テキストを
/// 含まない内容(ネストしたテーブル・置換要素等)からは求められないため`None`を
/// 返す(既知の簡略化、呼び出し側で`bottom`相当にフォールバックする)。
fn own_baseline_offset(cell: &LaidOutBox, fonts: &FontCollection) -> Option<f32> {
    let absolute = first_baseline_absolute_y(cell, fonts)?;
    Some(absolute - cell.layout.content.y)
}

/// `b`の内容を文書順に深さ優先で辿り、最初に見つかったテキスト行のベースライン
/// の絶対Y座標(`b`と同じ座標空間)を返す。
fn first_baseline_absolute_y(b: &LaidOutBox, fonts: &FontCollection) -> Option<f32> {
    match &b.content {
        LaidOutContent::Inline(lines) => lines.iter().find_map(|line| {
            let first_run = line.runs.first()?;
            let font = fonts.get(first_run.font_index)?;
            let offset = font.baseline_offset(first_run.font_size, line.rect.height);
            Some(line.rect.y + offset)
        }),
        LaidOutContent::Blocks(children) => children
            .iter()
            .find_map(|child| first_baseline_absolute_y(child, fonts)),
        // ネストしたテーブル・flex・grid・置換要素はベースラインを提供しない
        // (既知の簡略化)。
        LaidOutContent::Table(_)
        | LaidOutContent::Flex(_)
        | LaidOutContent::Grid(_)
        | LaidOutContent::Image(_) => None,
    }
}

/// 行番号+実際の開始列・終了列(exclusive)を持つ、グリッド上の1セルへの参照。
/// CSS2.1 §17.2「テーブルグリッド構築」の簡略版。
struct GridCell<'a> {
    cell: &'a TableCell,
    row_index: usize,
    col_start: usize,
    col_end: usize,
}

/// `rows`から、rowspan/colspanのoccupancy(rowspanで埋まっている列を後続行が
/// スキップする)を考慮したグリッド配置を求める。戻り値は行ごとの`GridCell`
/// 一覧(外側の`Vec`が行、内側がその行に属するセル)。rowspanが全て1の場合、列
/// カーソルを`col += cell.colspan`で単純に進める走査と完全に同一の結果を
/// 返すため、既存の(rowspanを使わない)テストへの後方互換性が保たれる。
fn build_table_grid(rows: &[TableRow], column_count: usize) -> Vec<Vec<GridCell<'_>>> {
    // occupied[col]: その列を占有しているrowspanの残り行数(0なら空き)。
    let mut occupied = vec![0usize; column_count];
    let mut grid = Vec::with_capacity(rows.len());

    for (row_index, row) in rows.iter().enumerate() {
        let mut row_cells = Vec::with_capacity(row.cells.len());
        let mut col = 0usize;
        for cell in &row.cells {
            while col < column_count && occupied[col] > 0 {
                col += 1;
            }
            let col_start = col;
            let col_end = (col_start + cell.colspan).min(column_count);
            for slot in &mut occupied[col_start..col_end] {
                *slot = cell.rowspan;
            }
            row_cells.push(GridCell {
                cell,
                row_index,
                col_start,
                col_end,
            });
            col = col_end;
        }
        grid.push(row_cells);
        // この行の処理が終わったので、rowspanの残数を1減らす
        // (この行分を消費、0になった列は次の行から再び空く)。
        for slot in &mut occupied {
            *slot = slot.saturating_sub(1);
        }
    }

    grid
}

/// `table-layout: fixed`用の列幅決定(CSS2.1 §17.5.2.1の簡略版)。
/// `<col>`由来のヒント(`column_hints`)を最優先し、次に最初の行のセルの明示
/// `width`(px/%)をそのセルが占める列の合計幅とし(colspanで複数列にまたがる
/// 場合は列数で均等に分割する)、どちらも無い列は残りの幅を均等配分する。
/// 1行目に無い列(1行目のcolspan合計が`column_count`に満たない場合)も均等
/// 配分の対象に含める。内容の測定は一切行わない。
fn compute_fixed_column_widths(
    grid: &[Vec<GridCell<'_>>],
    styles: &HashMap<NodeId, ComputedStyle>,
    column_hints: &[Option<f32>],
    column_count: usize,
    containing_width: f32,
) -> Vec<f32> {
    let mut widths: Vec<Option<f32>> = vec![None; column_count];

    if let Some(first_row) = grid.first() {
        for gc in first_row {
            if gc.col_end > gc.col_start {
                let cell_style = box_style(&gc.cell.content, styles);
                if let Some(resolved) = fixed_cell_width(&cell_style, containing_width) {
                    let share = resolved / (gc.col_end - gc.col_start) as f32;
                    for w in &mut widths[gc.col_start..gc.col_end] {
                        *w = Some(share);
                    }
                }
            }
        }
    }

    // `<col>`の指定は最初の行のセルより優先する。
    for (i, hint) in column_hints.iter().enumerate().take(column_count) {
        if let Some(hint) = hint {
            widths[i] = Some(*hint);
        }
    }

    let specified_sum: f32 = widths.iter().filter_map(|w| *w).sum();
    let auto_count = widths.iter().filter(|w| w.is_none()).count();
    let remaining = (containing_width - specified_sum).max(0.0);
    let auto_share = if auto_count > 0 {
        remaining / auto_count as f32
    } else {
        0.0
    };

    widths.iter().map(|w| w.unwrap_or(auto_share)).collect()
}

/// 各列の使用幅を求める。セルの内容から求めた「自然な幅」の列ごとの最大値を、
/// containing widthちょうどに収まるよう比例縮尺する。
///
/// `<col>`由来のヒント(`column_hints`)がある列はその幅で確定させ、残りの幅を
/// ヒントの無い列へ自然幅に比例して配分する。
fn compute_column_widths(
    grid: &[Vec<GridCell<'_>>],
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    column_hints: &[Option<f32>],
    column_count: usize,
    containing_width: f32,
) -> Vec<f32> {
    let mut natural = vec![0.0f32; column_count];

    // 1パス目: colspan=1のセルだけで各列の自然幅の最大値を求める。
    for row_cells in grid {
        for gc in row_cells {
            if gc.col_end - gc.col_start == 1 {
                natural[gc.col_start] =
                    natural[gc.col_start].max(natural_cell_width(gc.cell, styles, fonts));
            }
        }
    }

    // 2パス目: colspanをまたぐセルについて、またぐ列の自然幅合計がセル自身の
    // 自然幅に満たなければ、不足分をまたぐ列へ均等に上乗せする。
    for row_cells in grid {
        for gc in row_cells {
            if gc.col_end - gc.col_start > 1 {
                let span_natural_sum: f32 = natural[gc.col_start..gc.col_end].iter().sum();
                let cell_natural = natural_cell_width(gc.cell, styles, fonts);
                if cell_natural > span_natural_sum {
                    let deficit = cell_natural - span_natural_sum;
                    let share = deficit / (gc.col_end - gc.col_start) as f32;
                    for w in &mut natural[gc.col_start..gc.col_end] {
                        *w += share;
                    }
                }
            }
        }
    }

    let has_hint = column_hints
        .iter()
        .take(column_count)
        .any(|hint| hint.is_some());
    if has_hint {
        return distribute_with_column_hints(
            &natural,
            column_hints,
            column_count,
            containing_width,
        );
    }

    let natural_sum: f32 = natural.iter().sum();
    if natural_sum > 0.0 {
        let scale = containing_width / natural_sum;
        natural.iter().map(|w| w * scale).collect()
    } else {
        vec![containing_width / column_count as f32; column_count]
    }
}

/// `table-layout: fixed`で、1行目のセルが列幅として与える指定値。
///
/// * `width`指定あり → `min-width`/`max-width`でクランプした値
/// * `width: auto`で`min-width`のみ指定 → `min-width`をその列の指定幅とする
/// * どちらも無い(`max-width`だけの指定を含む) → `None`(残り幅の均等配分に委ねる)
fn fixed_cell_width(cell_style: &ComputedStyle, containing_width: f32) -> Option<f32> {
    match cell_style.width {
        LengthPercentageOrAuto::LengthPercentage(lp) => Some(clamp_used_width(
            cell_style,
            containing_width,
            0.0,
            0.0,
            resolve_lp(lp, containing_width),
        )),
        // `min-width`の初期値は`0`。0のときは「指定なし」として扱う。
        LengthPercentageOrAuto::Auto => {
            let min = resolve_lp(cell_style.min_width, containing_width);
            (min > 0.0).then_some(min)
        }
    }
}

/// `<col>`のヒントがある列を確定させ、残りをヒントの無い列へ自然幅に比例して
/// 配分する。ヒントの合計が`containing_width`を超える場合はヒントのある
/// 列だけを比例縮小して収める。
fn distribute_with_column_hints(
    natural: &[f32],
    column_hints: &[Option<f32>],
    column_count: usize,
    containing_width: f32,
) -> Vec<f32> {
    let hint_of = |i: usize| column_hints.get(i).copied().flatten();
    let hint_sum: f32 = (0..column_count).filter_map(hint_of).sum();

    if hint_sum > containing_width && hint_sum > 0.0 {
        let scale = containing_width / hint_sum;
        return (0..column_count)
            .map(|i| hint_of(i).map(|w| w * scale).unwrap_or(0.0))
            .collect();
    }

    let auto_natural_sum: f32 = (0..column_count)
        .filter(|&i| hint_of(i).is_none())
        .map(|i| natural[i])
        .sum();
    let remaining = (containing_width - hint_sum).max(0.0);

    (0..column_count)
        .map(|i| match hint_of(i) {
            Some(w) => w,
            None if auto_natural_sum > 0.0 => remaining * natural[i] / auto_natural_sum,
            None => {
                let auto_count = (0..column_count).filter(|&i| hint_of(i).is_none()).count();
                remaining / auto_count as f32
            }
        })
        .collect()
}

/// セル1つの「自然な幅」(内容を折り返し無しで並べた幅+パディング+ボーダー)。
///
/// セル自身の`min-width`/`max-width`はここでクランプする形で反映する。列の
/// 自然幅はクランプ済みの値の最大値になるが、その後の比例縮尺(表を紙幅に
/// 収める処理)は従来どおり行うため、最終列幅は`min-width`を保証しない。
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
    // クランプはcontent幅に対して行い(min/maxの指定値はcontent-box基準)、
    // padding/borderはその後に足す。min/maxのパーセンテージ基準はこの時点では
    // 未定のため0を基準に解決する(paddingと同じ簡略化)。
    let content_natural = measure_natural_content_width(&cell.content.content, styles, fonts);
    let clamped = clamp_used_width(
        &style,
        0.0,
        padding.left + padding.right,
        border.left + border.right,
        content_natural,
    );
    clamped + padding.left + padding.right + border.left + border.right
}

/// ボックスの内容を折り返し無しでレイアウトした場合の自然な幅を測る。
/// テーブルの自動列幅アルゴリズムに加え、`layout::flex`のtaffy採寸ブリッジ
/// (`available_space`が`MinContent`/`MaxContent`の場合)からも共有で使う。
pub(super) fn measure_natural_content_width(
    content: &BoxContent,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
) -> f32 {
    match content {
        BoxContent::Inline(spans) => {
            let lines =
                layout_inline_content(spans, styles, fonts, UNCONSTRAINED_WIDTH, 0.0, 0.0, None);
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
        // ネストしたテーブル・flex・gridの自然幅測定は非対応(既知の簡略化)。
        BoxContent::Table(_) | BoxContent::Flex(_) | BoxContent::Grid(_) => 0.0,
        BoxContent::Image(image_content) => image_content
            .attr_width
            .map(|w| w as f32)
            .or_else(|| image_content.image.as_ref().map(|img| img.width as f32))
            .unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::super::block::{layout_document, LaidOutBox};
    use super::super::box_tree::{build_box_tree, LayoutBox};
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

    /// `Table`(captionも含む)の中も辿る、テスト専用の`find_laid_out`。
    fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
        if b.node == Some(target) {
            return Some(b);
        }
        match &b.content {
            super::super::block::LaidOutContent::Blocks(children) => children
                .iter()
                .find_map(|child| find_laid_out(child, target)),
            super::super::block::LaidOutContent::Grid(grid) => grid
                .rows
                .iter()
                .flat_map(|row| &row.items)
                .find_map(|item| find_laid_out(item, target)),
            super::super::block::LaidOutContent::Table(table) => table
                .caption
                .as_deref()
                .and_then(|caption| find_laid_out(caption, target))
                .or_else(|| {
                    table
                        .rows
                        .iter()
                        .flat_map(|row| &row.cells)
                        .find_map(|cell| find_laid_out(cell, target))
                }),
            super::super::block::LaidOutContent::Flex(children) => children
                .iter()
                .find_map(|child| find_laid_out(child, target)),
            super::super::block::LaidOutContent::Inline(_)
            | super::super::block::LaidOutContent::Image(_) => None,
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
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        laid_table.rows[row]
            .cells
            .iter()
            .map(|c| c.layout.border_box().width)
            .collect()
    }

    fn cell_lefts(table: &LaidOutBox, row: usize) -> Vec<f32> {
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        laid_table.rows[row]
            .cells
            .iter()
            .map(|c| c.layout.border_box().x)
            .collect()
    }

    fn row_tops(table: &LaidOutBox) -> Vec<f32> {
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        laid_table
            .rows
            .iter()
            .map(|row| row.cells[0].layout.border_box().y)
            .collect()
    }

    fn row_cells(table: &LaidOutBox, row: usize) -> &[LaidOutBox] {
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        &laid_table.rows[row].cells
    }

    fn first_line_y(cell: &LaidOutBox) -> f32 {
        let super::super::block::LaidOutContent::Inline(lines) = &cell.content else {
            panic!("expected inline content");
        };
        lines[0].rect.y
    }

    /// `b`の内容を辿り、最初に見つかったネストしたテーブルの`LaidOutTable`を返す。
    fn find_nested_table(b: &LaidOutBox) -> Option<&LaidOutTable> {
        match &b.content {
            super::super::block::LaidOutContent::Table(table) => Some(table),
            super::super::block::LaidOutContent::Blocks(children)
            | super::super::block::LaidOutContent::Flex(children) => {
                children.iter().find_map(find_nested_table)
            }
            super::super::block::LaidOutContent::Grid(grid) => grid
                .rows
                .iter()
                .flat_map(|row| &row.items)
                .find_map(find_nested_table),
            super::super::block::LaidOutContent::Inline(_)
            | super::super::block::LaidOutContent::Image(_) => None,
        }
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
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        let total_width: f32 = laid_table.rows[0]
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
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        for cell in &laid_table.rows[0].cells {
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
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        let cells = &laid_table.rows[0].cells;
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
        let super::super::block::LaidOutContent::Table(laid_table) = &table.content else {
            panic!("expected a laid-out table");
        };
        assert!(laid_table.rows.is_empty());
        assert_eq!(table.layout.content.height, 0.0);
    }

    #[test]
    fn table_layout_fixed_uses_first_row_widths_and_ignores_content() {
        // 1行目のwidth指定(200px, 500px)がそのまま列幅になり、内容の長さは
        // 一切考慮されないはず(2行目の"x"のような短い内容があっても列幅は
        // 変わらない)。
        let table = layout_table_html(
            r#"<table style="table-layout: fixed;">
                <tr><td style="width: 200px;">a</td><td style="width: 500px;">a much much much longer piece of text</td></tr>
                <tr><td>x</td><td>x</td></tr>
            </table>"#,
            "",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 200.0).abs() < 0.5, "widths: {widths:?}");
        assert!((widths[1] - 500.0).abs() < 0.5, "widths: {widths:?}");
    }

    #[test]
    fn table_layout_fixed_distributes_remaining_width_to_auto_columns() {
        let table = layout_table_html(
            r#"<table style="table-layout: fixed;">
                <tr><td style="width: 100px;">a</td><td>b</td><td>c</td></tr>
            </table>"#,
            "body { margin: 0; }",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        // 残り600pxを2つのauto列で均等分割 = 300pxずつ。
        assert!((widths[0] - 100.0).abs() < 0.5, "widths: {widths:?}");
        assert!((widths[1] - 300.0).abs() < 0.5, "widths: {widths:?}");
        assert!((widths[2] - 300.0).abs() < 0.5, "widths: {widths:?}");
    }

    #[test]
    fn table_layout_fixed_splits_a_colspan_width_evenly_across_spanned_columns() {
        let table = layout_table_html(
            r#"<table style="table-layout: fixed;">
                <tr><td colspan="2" style="width: 400px;">a</td></tr>
            </table>"#,
            "",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        // 400pxを2列で均等分割 = 200pxずつ、colspanセル自体の幅は合計400px。
        assert!((widths[0] - 400.0).abs() < 0.5, "widths: {widths:?}");
    }

    #[test]
    fn border_spacing_adds_horizontal_gaps_between_and_around_columns() {
        // containing width 700px, border-spacing水平20px, 2列(等しい内容)。
        // 列に使える幅 = 700 - 20*3(列数+1個分の隙間) = 640 → 320pxずつ。
        let table = layout_table_html(
            "<table><tr><td>identical</td><td>identical</td></tr></table>",
            "body { margin: 0; } table { border-spacing: 20px 0; }",
            700.0,
        );
        let widths = cell_widths(&table, 0);
        let lefts = cell_lefts(&table, 0);
        assert!((widths[0] - 320.0).abs() < 0.5, "widths: {widths:?}");
        assert!((widths[1] - 320.0).abs() < 0.5, "widths: {widths:?}");
        assert!(
            (lefts[0] - 20.0).abs() < 0.5,
            "the first column should be inset by one spacing unit: {lefts:?}"
        );
        assert!(
            (lefts[1] - 360.0).abs() < 0.5,
            "the second column should start after column0 + 2 spacing units: {lefts:?}"
        );
    }

    #[test]
    fn border_spacing_adds_vertical_gaps_between_and_around_rows() {
        let table = layout_table_html(
            r#"<table>
                <tr><td style="height: 30px;">a</td></tr>
                <tr><td style="height: 30px;">b</td></tr>
            </table>"#,
            "body { margin: 0; } table { border-spacing: 0 15px; }",
            700.0,
        );
        let tops = row_tops(&table);
        assert!(
            (tops[0] - 15.0).abs() < 0.5,
            "the first row should be inset by one spacing unit: {tops:?}"
        );
        assert!(
            (tops[1] - 60.0).abs() < 0.5,
            "the second row should start after row0(30px) + 2 spacing units: {tops:?}"
        );
        // 全体の高さにも前後の`v_spacing`が含まれるはず: 15+30+15+30+15=105。
        assert!(
            (table.layout.content.height - 105.0).abs() < 0.5,
            "table height should include leading/trailing spacing: {}",
            table.layout.content.height
        );
    }

    #[test]
    fn border_collapse_forces_border_spacing_to_zero() {
        // `border-spacing`を明示していても`border-collapse: collapse`では
        // 無視され、セルは隙間なく隣接するはず(両者は排他)。
        let table = layout_table_html(
            "<table><tr><td>a</td><td>b</td></tr></table>",
            "body { margin: 0; } table { border-spacing: 20px; border-collapse: collapse; }",
            700.0,
        );
        let lefts = cell_lefts(&table, 0);
        let widths = cell_widths(&table, 0);
        assert!(
            lefts[0].abs() < 0.5,
            "with collapse the first column should touch the table's edge: {lefts:?}"
        );
        assert!(
            (lefts[1] - (lefts[0] + widths[0])).abs() < 0.5,
            "with collapse adjacent cells should touch with no gap: lefts={lefts:?} widths={widths:?}"
        );
    }

    #[test]
    fn vertical_align_top_keeps_content_flush_with_the_row_top() {
        let table = layout_table_html(
            r#"<table>
                <tr><td style="height: 10px;">a</td><td style="height: 80px;">b</td></tr>
            </table>"#,
            "body { margin: 0; } td { vertical-align: top; }",
            700.0,
        );
        for cell in row_cells(&table, 0) {
            assert_eq!(
                first_line_y(cell),
                0.0,
                "top-aligned content should stay flush with the row top"
            );
        }
    }

    #[test]
    fn vertical_align_bottom_pushes_the_shorter_cells_content_to_the_bottom() {
        let table = layout_table_html(
            r#"<table>
                <tr><td style="height: 10px;">a</td><td style="height: 80px;">b</td></tr>
            </table>"#,
            "body { margin: 0; } td { vertical-align: bottom; }",
            700.0,
        );
        let cells = row_cells(&table, 0);
        assert!(
            first_line_y(&cells[1]).abs() < 0.5,
            "the tallest cell defines the row height so its own content shouldn't shift: {}",
            first_line_y(&cells[1])
        );
        assert!(
            (first_line_y(&cells[0]) - 70.0).abs() < 0.5,
            "the shorter cell's content should be pushed down by the full deficit(80-10=70px): {}",
            first_line_y(&cells[0])
        );
    }

    #[test]
    fn vertical_align_middle_centers_the_shorter_cells_content() {
        let table = layout_table_html(
            r#"<table>
                <tr><td style="height: 10px;">a</td><td style="height: 80px;">b</td></tr>
            </table>"#,
            "body { margin: 0; } td { vertical-align: middle; }",
            700.0,
        );
        let cells = row_cells(&table, 0);
        assert!(
            first_line_y(&cells[1]).abs() < 0.5,
            "the tallest cell defines the row height so its own content shouldn't shift: {}",
            first_line_y(&cells[1])
        );
        assert!(
            (first_line_y(&cells[0]) - 35.0).abs() < 0.5,
            "the shorter cell's content should be pushed down by half the deficit((80-10)/2=35px): {}",
            first_line_y(&cells[0])
        );
    }

    #[test]
    fn vertical_align_baseline_aligns_first_lines_of_cells_with_different_font_sizes() {
        // フォントサイズが異なる(=行の高さも異なる)セルどうしでも、テキストの
        // ベースライン自体は同じY座標に揃うはず(CSS2.1 17.5.3のbaseline揃え)。
        let table = layout_table_html(
            r#"<table>
                <tr><td style="font-size: 12px;">Ay</td><td style="font-size: 36px;">Ay</td></tr>
            </table>"#,
            "body { margin: 0; } td { vertical-align: baseline; }",
            700.0,
        );
        let fonts = test_fonts();
        let cells = row_cells(&table, 0);
        let baseline_y = |cell: &LaidOutBox| {
            let super::super::block::LaidOutContent::Inline(lines) = &cell.content else {
                panic!("expected inline content");
            };
            let run = lines[0].runs.first().expect("cell should have text");
            let font = fonts.get(run.font_index).expect("font should be loaded");
            lines[0].rect.y + font.baseline_offset(run.font_size, lines[0].rect.height)
        };

        let small_baseline = baseline_y(&cells[0]);
        let large_baseline = baseline_y(&cells[1]);
        assert!(
            (small_baseline - large_baseline).abs() < 0.5,
            "baseline-aligned cells should share the same baseline Y: small={small_baseline} large={large_baseline}"
        );
    }

    #[test]
    fn vertical_align_baseline_falls_back_to_bottom_for_content_without_a_baseline() {
        // ネストしたテーブルにはベースラインが無い(既知の簡略化)ため、
        // `bottom`相当にフォールバックするはず。
        let table = layout_table_html(
            r#"<table>
                <tr>
                    <td style="height: 80px;">a</td>
                    <td style="height: 10px;"><table><tr><td>nested</td></tr></table></td>
                </tr>
            </table>"#,
            "body { margin: 0; } td { vertical-align: baseline; }",
            700.0,
        );
        let cells = row_cells(&table, 0);
        let nested_top_y = find_nested_table(&cells[1])
            .expect("expected the outer cell to contain a nested table")
            .rows[0]
            .cells[0]
            .layout
            .border_box()
            .y;
        assert!(
            (nested_top_y - 70.0).abs() < 0.5,
            "content without a baseline should fall back to bottom alignment (deficit=80-10=70px): {nested_top_y}"
        );
    }

    fn grid_cell(colspan: usize, rowspan: usize) -> TableCell {
        TableCell {
            node: NodeId(0),
            colspan,
            rowspan,
            content: LayoutBox {
                node: None,
                content: BoxContent::Inline(Vec::new()),
                marker: None,
            },
        }
    }

    fn grid_row(cells: Vec<TableCell>) -> TableRow {
        TableRow {
            node: NodeId(0),
            cells,
            section: super::super::box_tree::TableSection::Body,
        }
    }

    /// `(col_start, col_end)`の一覧に変換して、グリッド構築結果を検証しやすくする。
    fn grid_spans(grid: &[Vec<GridCell<'_>>]) -> Vec<Vec<(usize, usize)>> {
        grid.iter()
            .map(|row| row.iter().map(|gc| (gc.col_start, gc.col_end)).collect())
            .collect()
    }

    #[test]
    fn build_table_grid_matches_the_naive_colspan_walk_when_rowspan_is_always_one() {
        let rows = vec![
            grid_row(vec![grid_cell(2, 1), grid_cell(1, 1)]),
            grid_row(vec![grid_cell(1, 1), grid_cell(1, 1), grid_cell(1, 1)]),
        ];
        let grid = build_table_grid(&rows, 3);
        assert_eq!(
            grid_spans(&grid),
            vec![vec![(0, 2), (2, 3)], vec![(0, 1), (1, 2), (2, 3)]]
        );
    }

    #[test]
    fn build_table_grid_skips_columns_occupied_by_a_rowspan_cell() {
        // 行0: col0がrowspan=2で2行分占有、col1は通常セル。
        // 行1: 1つしかセルが無いが、col0がまだrowspanで埋まっているため
        // col1から配置されるはず。
        // 行2: rowspanが解けているので、col0から配置し直せるはず。
        let rows = vec![
            grid_row(vec![grid_cell(1, 2), grid_cell(1, 1)]),
            grid_row(vec![grid_cell(1, 1)]),
            grid_row(vec![grid_cell(1, 1), grid_cell(1, 1)]),
        ];
        let grid = build_table_grid(&rows, 2);
        assert_eq!(
            grid_spans(&grid),
            vec![vec![(0, 1), (1, 2)], vec![(1, 2)], vec![(0, 1), (1, 2)]]
        );
    }

    #[test]
    fn build_table_grid_handles_rowspan_and_colspan_combined() {
        // 行0: col0..2をまたぐ(colspan=2)セルがrowspan=2で2行分占有。
        // 行1: col0/col1双方が埋まっているため、次のセルはcol2から配置される。
        let rows = vec![
            grid_row(vec![grid_cell(2, 2), grid_cell(1, 1)]),
            grid_row(vec![grid_cell(1, 1)]),
        ];
        let grid = build_table_grid(&rows, 3);
        assert_eq!(grid_spans(&grid), vec![vec![(0, 2), (2, 3)], vec![(2, 3)]]);
    }

    #[test]
    fn rowspan_cell_spans_the_full_height_of_the_rows_it_covers() {
        // "tall"(rowspan=2, 明示80px)が行0・行1の自然な高さ(それぞれ10pxの
        // セルしか無い)を上回るため、両方の行の高さが40pxずつに拡張され、
        // "tall"自身の高さはその合計(=80px)ちょうどになるはず。
        let table = layout_table_html(
            r#"<table>
                <tr><td rowspan="2" style="height: 80px;">tall</td><td style="height: 10px;">a</td></tr>
                <tr><td style="height: 10px;">b</td></tr>
            </table>"#,
            "body { margin: 0; }",
            700.0,
        );
        let row0 = row_cells(&table, 0);
        let row1 = row_cells(&table, 1);
        assert_eq!(row1.len(), 1, "row1 should only have its own single cell");

        assert!(
            (row0[0].layout.margin_box_height() - 80.0).abs() < 0.5,
            "the rowspan cell should span exactly the combined height of both rows: {}",
            row0[0].layout.margin_box_height()
        );
        assert!(
            (row0[1].layout.margin_box_height() - 40.0).abs() < 0.5,
            "row0's non-spanning cell should be stretched to row0's height(40px): {}",
            row0[1].layout.margin_box_height()
        );
        assert!(
            (row1[0].layout.border_box().y - 40.0).abs() < 0.5,
            "row1 should start after row0's height(40px): {}",
            row1[0].layout.border_box().y
        );
    }

    #[test]
    fn rowspan_cell_makes_the_following_row_skip_its_occupied_column() {
        // 行1のセルは1個しか無いが、col0が行0のrowspanセルに占有されている
        // ため、col1(行0の2列目と同じ列)に配置されるはず。
        let table = layout_table_html(
            r#"<table>
                <tr><td rowspan="2">tall</td><td>a</td></tr>
                <tr><td>b</td></tr>
            </table>"#,
            "body { margin: 0; }",
            700.0,
        );
        let row0_lefts = cell_lefts(&table, 0);
        let row1 = row_cells(&table, 1);
        assert!(
            (row1[0].layout.border_box().x - row0_lefts[1]).abs() < 0.5,
            "row1's single cell should land in column1 (same x as row0's second cell), not column0: row1_x={} row0_col1_x={}",
            row1[0].layout.border_box().x,
            row0_lefts[1]
        );
    }

    // ===== <colgroup>/<col>(列幅指定) =====

    #[test]
    fn col_width_fixes_the_column_width_in_auto_layout() {
        // border-spacingを0にして、列幅=セルのborder-box幅が直接比較できるようにする。
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col style="width: 100px;"><col></colgroup>
                 <tr><td>a</td><td>bbbbbbbbbbbbbbbb</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 100.0).abs() < 0.5, "got {widths:?}");
        // 残り幅は指定の無い列が全部もらう。
        assert!((widths[1] - 400.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn col_percentage_width_resolves_against_the_table_width() {
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col style="width: 20%;"><col></colgroup>
                 <tr><td>a</td><td>b</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 100.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn col_span_applies_the_same_width_to_several_columns() {
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col span="2" style="width: 50px;"><col></colgroup>
                 <tr><td>a</td><td>b</td><td>c</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 50.0).abs() < 0.5, "got {widths:?}");
        assert!((widths[1] - 50.0).abs() < 0.5, "got {widths:?}");
        assert!((widths[2] - 400.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn colgroup_span_without_col_children_defines_the_columns_itself() {
        let table = layout_table_html(
            r#"<table>
                 <colgroup span="2" style="width: 60px;"></colgroup>
                 <tr><td>a</td><td>b</td><td>c</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 60.0).abs() < 0.5, "got {widths:?}");
        assert!((widths[1] - 60.0).abs() < 0.5, "got {widths:?}");
        assert!((widths[2] - 380.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn columns_without_a_col_hint_share_the_rest_proportionally_to_their_content() {
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col style="width: 100px;"><col><col></colgroup>
                 <tr><td>a</td><td>short</td><td>a much much much longer cell</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 100.0).abs() < 0.5, "got {widths:?}");
        assert!(
            widths[2] > widths[1],
            "the column with more content should get more of the remaining width: {widths:?}"
        );
        let total: f32 = widths.iter().sum();
        assert!((total - 500.0).abs() < 1.0, "got {widths:?}");
    }

    #[test]
    fn col_hints_wider_than_the_table_are_scaled_down_to_fit() {
        // 指定の合計が使える幅を超えたら指定列だけを比例縮小する。
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col style="width: 600px;"><col style="width: 200px;"></colgroup>
                 <tr><td>a</td><td>b</td></tr>
               </table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            400.0,
        );
        let widths = cell_widths(&table, 0);
        assert!((widths[0] - 300.0).abs() < 0.5, "got {widths:?}");
        assert!((widths[1] - 100.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn col_width_takes_precedence_over_the_first_row_cell_in_fixed_layout() {
        let table = layout_table_html(
            r#"<table>
                 <colgroup><col style="width: 300px;"><col></colgroup>
                 <tr><td style="width: 100px;">a</td><td>b</td></tr>
               </table>"#,
            "body { margin: 0; } table { table-layout: fixed; border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        assert!(
            (widths[0] - 300.0).abs() < 0.5,
            "the <col> width must win over the first row cell: {widths:?}"
        );
        assert!((widths[1] - 200.0).abs() < 0.5, "got {widths:?}");
    }

    #[test]
    fn a_table_without_colgroup_keeps_the_previous_behaviour() {
        // 回帰確認: ヒントが1つも無ければ従来どおり全列を比例縮尺する。
        let table = layout_table_html(
            r#"<table><tr><td>a</td><td>a much much much longer cell</td></tr></table>"#,
            "body { margin: 0; } table { border-spacing: 0; }",
            500.0,
        );
        let widths = cell_widths(&table, 0);
        let total: f32 = widths.iter().sum();
        assert!((total - 500.0).abs() < 1.0, "got {widths:?}");
        assert!(widths[1] > widths[0], "got {widths:?}");
    }
}
