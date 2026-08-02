//! CSS Grid(`display: grid`)を、既存box treeのサブツリーとしてtaffyへ
//! ブリッジする。
//!
//! taffyへの橋渡し(採寸コールバック・座標変換の2パス方式)はFlexboxと完全に
//! 共通で、[`super::flex::layout_taffy_subtree`]に集約してある。この
//! モジュールが持つのは「CSSのgrid固有プロパティをtaffyの`Style`へ写す」
//! 部分と、「レイアウト結果を行帯(ページ分割の単位)へ分類する」部分。

use std::collections::HashMap;

use taffy as tf;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::{
    AlignItems, AlignSelf, ComputedStyle, GridArea, GridAutoFlow, GridLine, LengthPercentage,
    TrackBreadth, TrackComponent, TrackList, TrackSize,
};

use super::block::LaidOutBox;
use super::box_tree::GridBox;
use super::flex::{layout_taffy_subtree, TaffyMode};

/// レイアウト済みのグリッド。ページ分割の単位である「行帯」の列を持つ。
#[derive(Debug, Clone)]
pub struct LaidOutGrid {
    pub rows: Vec<LaidOutGridRow>,
}

/// グリッド1行分の帯。`top`/`bottom`は絶対y座標(他のレイアウト結果と
/// 同じ座標空間。`shift_box_y`が他の座標と一緒に平行移動する)。
#[derive(Debug, Clone)]
pub struct LaidOutGridRow {
    pub items: Vec<LaidOutBox>,
    pub top: f32,
    pub bottom: f32,
    /// この行帯の下端をまたぐアイテムがあるか。`true`ならここでページを
    /// 分割できない(テーブルの`rowspan`と同じ扱い)。
    pub spans_bottom: bool,
}

/// グリッドコンテナのcontent box内でアイテムをレイアウトする。
/// 返り値はレイアウト済みのグリッドと、コンテナの自然なcontent-box高さ。
pub(super) fn layout_grid(
    grid: &GridBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    container_style: &ComputedStyle,
    content_width: f32,
    content_x: f32,
    content_y: f32,
) -> (LaidOutGrid, f32) {
    let result = layout_taffy_subtree(
        &grid.items,
        styles,
        fonts,
        container_style,
        content_width,
        content_x,
        content_y,
        TaffyMode::Grid,
    );

    let rows = group_into_rows(result.items, result.row_tracks.as_ref(), content_y);
    (LaidOutGrid { rows }, result.container_height)
}

/// レイアウト済みアイテムを行帯へ分類する。
///
/// taffyの`DetailedGridInfo::items`はグリッドの配置アルゴリズム内部の順序で
/// 並んでおり、リーフの順序と対応する保証が無い。そのため行トラックの
/// 使用サイズから求めた帯の範囲と、各アイテムの実際のy座標を突き合わせて
/// 幾何的に判定する(アイテムの上端が入る帯に属させ、下端が帯を越えていれば
/// `spans_bottom`を立てる)。
fn group_into_rows(
    items: Vec<LaidOutBox>,
    row_tracks: Option<&super::flex::GridRowTracks>,
    content_y: f32,
) -> Vec<LaidOutGridRow> {
    let Some(tracks) = row_tracks.filter(|tracks| !tracks.sizes.is_empty()) else {
        // 行トラック情報が取れない場合は全体を1つの帯として扱う
        // (=分割しない。従来のflexコンテナと同じ挙動)。
        let (top, bottom) = items_vertical_extent(&items, content_y);
        return vec![LaidOutGridRow {
            items,
            top,
            bottom,
            spans_bottom: false,
        }];
    };

    // 行帯の範囲。taffyのトラック配列は
    // [ガター, トラック, ガター, ..., ガター]の並びなので、i番目のトラックの
    // 開始位置は「先頭からi+1個のガター + i個のトラック」の合計になる。
    let mut bands: Vec<(f32, f32)> = Vec::with_capacity(tracks.sizes.len());
    // content boxの上端(絶対座標)を起点にする。
    let mut offset = content_y;
    for (i, size) in tracks.sizes.iter().enumerate() {
        offset += tracks.gutters.get(i).copied().unwrap_or(0.0);
        bands.push((offset, offset + size));
        offset += size;
    }

    let mut rows: Vec<LaidOutGridRow> = bands
        .iter()
        .map(|(top, bottom)| LaidOutGridRow {
            items: Vec::new(),
            top: *top,
            bottom: *bottom,
            spans_bottom: false,
        })
        .collect();

    for item in items {
        let margin_box_top = item.layout.content.y
            - item.layout.padding.top
            - item.layout.border.top
            - item.layout.margin.top;
        let margin_box_bottom = margin_box_top + item.layout.margin_box_height();

        // 上端が属する帯(見つからなければ最後の帯)。
        let index = bands
            .iter()
            .position(|(top, bottom)| margin_box_top < *bottom || margin_box_top <= *top)
            .unwrap_or(bands.len() - 1);
        // 下端が自分の帯を越えていれば、その帯の境界では分割できない。
        if margin_box_bottom > bands[index].1 + BAND_EPSILON {
            rows[index].spans_bottom = true;
        }
        rows[index].items.push(item);
    }

    // アイテムが1つも無い帯(空行)も高さを持つので残す。
    rows
}

/// 帯の境界判定に使う許容誤差(px)。taffyが返す座標は浮動小数のため、
/// ちょうど境界に接するアイテムを「またいでいる」と誤判定しないようにする。
const BAND_EPSILON: f32 = 0.01;

/// 行トラック情報が無い場合のフォールバック: アイテム全体の上端/下端(絶対座標)。
fn items_vertical_extent(items: &[LaidOutBox], content_y: f32) -> (f32, f32) {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for item in items {
        let item_top = item.layout.content.y
            - item.layout.padding.top
            - item.layout.border.top
            - item.layout.margin.top;
        top = top.min(item_top);
        bottom = bottom.max(item_top + item.layout.margin_box_height());
    }
    if items.is_empty() {
        (content_y, content_y)
    } else {
        (top, bottom)
    }
}

/// グリッドコンテナのtaffy `Style`。
pub(super) fn container_taffy_style(style: &ComputedStyle, content_width: f32) -> tf::Style {
    tf::Style {
        display: tf::Display::Grid,
        grid_template_columns: map_track_list(&style.grid_template_columns),
        grid_template_rows: map_track_list(&style.grid_template_rows),
        grid_auto_columns: map_auto_tracks(&style.grid_auto_columns),
        grid_auto_rows: map_auto_tracks(&style.grid_auto_rows),
        grid_auto_flow: map_auto_flow(style.grid_auto_flow),
        grid_template_areas: map_template_areas(&style.grid_template_areas),
        grid_template_column_names: map_line_names(&style.grid_template_columns),
        grid_template_row_names: map_line_names(&style.grid_template_rows),
        justify_content: Some(super::flex::map_justify_content(style.justify_content)),
        align_content: Some(super::flex::map_align_content(style.align_content)),
        // Gridでは`justify-items`/`align-items`の両方が意味を持つ。
        justify_items: Some(map_align_items(style.justify_items)),
        align_items: Some(super::flex::map_align_items(style.align_items)),
        gap: tf::Size {
            width: super::flex::map_length_percentage(style.column_gap),
            height: super::flex::map_length_percentage(style.row_gap),
        },
        size: tf::Size {
            width: tf::Dimension::length(content_width),
            height: super::flex::map_dimension(style.height),
        },
        min_size: tf::Size {
            width: super::flex::map_length_percentage_dimension(style.min_width),
            height: super::flex::map_length_percentage_dimension(style.min_height),
        },
        max_size: tf::Size {
            width: super::flex::map_max_size(style.max_width),
            height: super::flex::map_max_size(style.max_height),
        },
        aspect_ratio: style.aspect_ratio.ratio,
        ..Default::default()
    }
}

/// グリッドアイテムのtaffy `Style`。
pub(super) fn item_taffy_style(style: &ComputedStyle) -> tf::Style {
    let mut base = super::flex::item_taffy_style(style);
    base.grid_row = tf::Line {
        start: map_grid_line(&style.grid_row_start),
        end: map_grid_line(&style.grid_row_end),
    };
    base.grid_column = tf::Line {
        start: map_grid_line(&style.grid_column_start),
        end: map_grid_line(&style.grid_column_end),
    };
    base.justify_self = map_align_self(style.justify_self);
    base
}

fn map_track_list(list: &TrackList) -> Vec<tf::GridTemplateComponent<String>> {
    list.components
        .iter()
        .map(|component| match component {
            TrackComponent::Single(size) => {
                tf::GridTemplateComponent::Single(map_track_size(*size))
            }
            TrackComponent::Repeat {
                count,
                tracks,
                line_names,
            } => tf::GridTemplateComponent::Repeat(tf::GridTemplateRepetition {
                count: match count {
                    crate::style::RepeatCount::Count(n) => tf::RepetitionCount::Count(*n),
                    crate::style::RepeatCount::AutoFill => tf::RepetitionCount::AutoFill,
                    crate::style::RepeatCount::AutoFit => tf::RepetitionCount::AutoFit,
                },
                tracks: tracks.iter().map(|size| map_track_size(*size)).collect(),
                line_names: line_names.clone(),
            }),
        })
        .collect()
}

/// `[name]`で書かれたライン名。taffyは「トラック境界ごとに1リスト」の形で持つ。
fn map_line_names(list: &TrackList) -> Vec<Vec<String>> {
    list.line_names.clone()
}

fn map_auto_tracks(sizes: &[TrackSize]) -> Vec<tf::TrackSizingFunction> {
    sizes.iter().map(|size| map_track_size(*size)).collect()
}

fn map_track_size(size: TrackSize) -> tf::TrackSizingFunction {
    match size {
        TrackSize::Breadth(breadth) => tf::TrackSizingFunction {
            min: map_min_breadth(breadth),
            max: map_max_breadth(breadth),
        },
        TrackSize::MinMax(min, max) => tf::TrackSizingFunction {
            min: map_min_breadth(min),
            max: map_max_breadth(max),
        },
        TrackSize::FitContent(lp) => tf::TrackSizingFunction {
            min: tf::MinTrackSizingFunction::auto(),
            max: match lp {
                LengthPercentage::Length(px) => tf::MaxTrackSizingFunction::fit_content_px(px),
                LengthPercentage::Percentage(v) => {
                    tf::MaxTrackSizingFunction::fit_content_percent(v)
                }
                // calcのトラックサイズはパーサが拒否するのでここへは来ない。
                LengthPercentage::Calc { px, .. } => tf::MaxTrackSizingFunction::fit_content_px(px),
            },
        },
    }
}

/// `<track-breadth>`をtaffyの最小トラックサイズへ。`fr`は最小側では
/// `auto`扱い(CSS仕様: `1fr`は`minmax(auto, 1fr)`と等価)。
fn map_min_breadth(breadth: TrackBreadth) -> tf::MinTrackSizingFunction {
    match breadth {
        TrackBreadth::Length(px) => tf::MinTrackSizingFunction::length(px),
        TrackBreadth::Percentage(v) => tf::MinTrackSizingFunction::percent(v),
        TrackBreadth::Fr(_) | TrackBreadth::Auto => tf::MinTrackSizingFunction::auto(),
        TrackBreadth::MinContent => tf::MinTrackSizingFunction::min_content(),
        TrackBreadth::MaxContent => tf::MinTrackSizingFunction::max_content(),
    }
}

fn map_max_breadth(breadth: TrackBreadth) -> tf::MaxTrackSizingFunction {
    match breadth {
        TrackBreadth::Length(px) => tf::MaxTrackSizingFunction::length(px),
        TrackBreadth::Percentage(v) => tf::MaxTrackSizingFunction::percent(v),
        TrackBreadth::Fr(v) => tf::MaxTrackSizingFunction::fr(v),
        TrackBreadth::Auto => tf::MaxTrackSizingFunction::auto(),
        TrackBreadth::MinContent => tf::MaxTrackSizingFunction::min_content(),
        TrackBreadth::MaxContent => tf::MaxTrackSizingFunction::max_content(),
    }
}

fn map_auto_flow(flow: GridAutoFlow) -> tf::GridAutoFlow {
    match flow {
        GridAutoFlow::Row => tf::GridAutoFlow::Row,
        GridAutoFlow::Column => tf::GridAutoFlow::Column,
        GridAutoFlow::RowDense => tf::GridAutoFlow::RowDense,
        GridAutoFlow::ColumnDense => tf::GridAutoFlow::ColumnDense,
    }
}

fn map_template_areas(areas: &[GridArea]) -> Vec<tf::GridTemplateArea<String>> {
    areas
        .iter()
        .map(|area| tf::GridTemplateArea {
            name: area.name.clone(),
            row_start: area.row_start,
            row_end: area.row_end,
            column_start: area.column_start,
            column_end: area.column_end,
        })
        .collect()
}

fn map_grid_line(line: &GridLine) -> tf::GridPlacement<String> {
    match line {
        GridLine::Auto => tf::GridPlacement::Auto,
        GridLine::Line(n) => tf::GridPlacement::Line((*n).into()),
        GridLine::Span(n) => tf::GridPlacement::Span(*n),
        GridLine::Named(name) => tf::GridPlacement::NamedLine(name.clone(), 1),
        GridLine::NamedSpan(name, n) => tf::GridPlacement::NamedSpan(name.clone(), *n),
    }
}

/// `justify-items`はGridのインライン軸方向のアイテム配置。値の集合は
/// `align-items`と共有する。
fn map_align_items(items: AlignItems) -> tf::AlignItems {
    super::flex::map_align_items(items)
}

fn map_align_self(align: AlignSelf) -> Option<tf::AlignSelf> {
    super::flex::map_align_self(align)
}
