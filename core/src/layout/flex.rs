//! Flexbox(`display: flex`)を、既存box treeのサブツリーとしてtaffyへ
//! ブリッジする。
//!
//! taffyは自前でノードツリー(`TaffyTree`)を持つ設計のため、flexコンテナ・
//! 各アイテムに対応するtaffyのリーフノードを都度組み立て、`compute_layout_with_measure`
//! で1回だけレイアウトを計算する。テキスト等の内在サイズが必要なリーフには
//! 採寸(measure)コールバックを渡し、その中で既存のブロック/インライン/テーブル
//! レイアウト関数を呼んで実測する。計算結果(各アイテムの確定した位置・サイズ)
//! は、`layout_box_with_forced_size`でもう一度実際のレイアウトを行うことで
//! `LaidOutBox`へ変換する(2パス方式)。
//!
//! taffyの型は自前の同名CSS型(`crate::style::FlexDirection`等)と衝突するため
//! `tf`という別名で参照する。

use std::collections::HashMap;
use std::rc::Rc;

use taffy as tf;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, ComputedStyle, FlexBasis, FlexDirection,
    FlexWrap, JustifyContent, LengthPercentage, LengthPercentageOrAuto, MaxSize,
};

use super::block::{
    box_style, layout_box_with_forced_size_ignoring_positioned,
    layout_box_with_forced_width_ignoring_positioned, resolve_border, resolve_padding, LaidOutBox,
};
use super::box_tree::{FlexBox, LayoutBox};
use super::float_ctx::FloatContext;
use super::table::measure_natural_content_width;

/// flexコンテナのcontent box内(`content_x`/`content_y`起点、幅`content_width`)
/// でflexアイテム群をレイアウトする。返り値はレイアウト済みの各アイテムと、
/// コンテナの自然な(内容に基づく)content-box高さ(呼び出し側`block.rs`が
/// 明示`height`指定で上書きする前の値、`layout_table`と同じ役割分担)。
pub(super) fn layout_flex(
    flex: &FlexBox,
    styles: &HashMap<NodeId, Rc<ComputedStyle>>,
    fonts: &FontCollection,
    container_style: &ComputedStyle,
    content_width: f32,
    content_x: f32,
    content_y: f32,
) -> (Vec<LaidOutBox>, f32) {
    let result = layout_taffy_subtree(
        &flex.items,
        styles,
        fonts,
        container_style,
        content_width,
        content_x,
        content_y,
        TaffyMode::Flex,
    );
    (result.items, result.container_height)
}

/// taffyへ委譲するレイアウトの種類。コンテナ/アイテムへ渡す`Style`だけが
/// 変わり、採寸ブリッジと座標変換は共通。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaffyMode {
    Flex,
    Grid,
}

/// [`layout_taffy_subtree`]の結果。
pub(super) struct TaffySubtreeLayout {
    pub items: Vec<LaidOutBox>,
    /// コンテナの自然な(内容に基づく)content-box高さ。
    pub container_height: f32,
    /// Gridのときのみ、行トラックの使用サイズとガター(ページ分割用)。
    pub row_tracks: Option<GridRowTracks>,
}

/// グリッドの行トラック情報(taffyの`DetailedGridInfo`由来)。
/// `gutters`はトラックの前後に1つずつ入るため`sizes.len() + 1`要素。
pub(super) struct GridRowTracks {
    pub sizes: Vec<f32>,
    pub gutters: Vec<f32>,
}

/// flex/gridのアイテム群をtaffyでレイアウトし、既存の`LaidOutBox`へ変換する
/// (2パス方式)。
#[allow(clippy::too_many_arguments)]
pub(super) fn layout_taffy_subtree(
    flex_items: &[LayoutBox],
    styles: &HashMap<NodeId, Rc<ComputedStyle>>,
    fonts: &FontCollection,
    container_style: &ComputedStyle,
    content_width: f32,
    content_x: f32,
    content_y: f32,
    mode: TaffyMode,
) -> TaffySubtreeLayout {
    if flex_items.is_empty() {
        return TaffySubtreeLayout {
            items: Vec::new(),
            container_height: 0.0,
            row_tracks: None,
        };
    }

    let mut tree: tf::TaffyTree<usize> = tf::TaffyTree::new();

    let leaves: Vec<tf::NodeId> = flex_items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let item_style = box_style(item, styles);
            let leaf_style = match mode {
                TaffyMode::Flex => item_taffy_style(&item_style),
                TaffyMode::Grid => super::grid::item_taffy_style(&item_style),
            };
            tree.new_leaf_with_context(leaf_style, index)
                .expect("taffyへのリーフノード追加は失敗しない")
        })
        .collect();

    let root_style = match mode {
        TaffyMode::Flex => container_taffy_style(container_style, content_width),
        TaffyMode::Grid => super::grid::container_taffy_style(container_style, content_width),
    };
    let root = tree
        .new_with_children(root_style, &leaves)
        .expect("taffyへのルートノード追加は失敗しない");

    tree.compute_layout_with_measure(
        root,
        tf::Size {
            width: tf::AvailableSpace::Definite(content_width),
            height: tf::AvailableSpace::MaxContent,
        },
        |known_dimensions, available_space, _node_id, node_context, _style| {
            let Some(&mut index) = node_context else {
                return tf::Size::ZERO;
            };
            let item = &flex_items[index];
            let item_style = box_style(item, styles);

            // measureが返すのはcontent-box基準のサイズ(taffy自身がpadding/borderを
            // 加算する、`compute::leaf::compute_leaf_layout`で実測済みの規約)。
            let width = known_dimensions.width.unwrap_or_else(|| {
                match available_space.width {
                    // 「使える幅」が確定していても、内容がそれより狭ければ
                    // 内容幅を返す。ここで常に`w`を返すと、内容幅に縮むべき
                    // ケース(Gridの`justify-items: start`等)で常にトラック
                    // 幅いっぱいになってしまう。
                    tf::AvailableSpace::Definite(w) => {
                        measure_natural_content_width(&item.content, styles, fonts).min(w)
                    }
                    // min-contentとmax-contentは区別しない(既知の簡略化)。
                    tf::AvailableSpace::MinContent | tf::AvailableSpace::MaxContent => {
                        measure_natural_content_width(&item.content, styles, fonts)
                    }
                }
            });

            let height = known_dimensions.height.unwrap_or_else(|| {
                // パディング/ボーダーはCSS仕様通り常に「containing blockの幅」
                // (=flexコンテナのcontent_width)基準で解決する(水平・垂直とも)。
                let padding = resolve_padding(&item_style, content_width);
                let border = resolve_border(&item_style);
                let outer_width = width + padding.left + padding.right + border.left + border.right;

                let mut float_ctx = FloatContext::new();
                let laid = layout_box_with_forced_width_ignoring_positioned(
                    item,
                    styles,
                    fonts,
                    outer_width,
                    width,
                    &mut float_ctx,
                    0.0,
                    0.0,
                );
                laid.layout.content.height
            });

            tf::Size { width, height }
        },
    )
    .expect("compute_layout_with_measureは失敗しない");

    let mut result = Vec::with_capacity(flex_items.len());
    for (index, item) in flex_items.iter().enumerate() {
        let leaf = leaves[index];
        let item_layout = tree
            .layout(leaf)
            .expect("直前にcompute_layout_with_measureで計算済み");
        let item_style = box_style(item, styles);

        // taffyのLayout.size/padding/borderはborder-box前提(スパイクで実測確認
        // 済み)。content-box幅・高さへ変換する。
        let content_w = (item_layout.size.width
            - item_layout.padding.left
            - item_layout.padding.right
            - item_layout.border.left
            - item_layout.border.right)
            .max(0.0);
        let content_h = (item_layout.size.height
            - item_layout.padding.top
            - item_layout.padding.bottom
            - item_layout.border.top
            - item_layout.border.bottom)
            .max(0.0);

        // taffyのlocationはborder-box原点(marginは既に位置に織り込み済み、
        // スパイクで実測確認済み: margin-left: 10pxのリーフはlocation.x=10)。
        // `layout_box_with_forced_size`のx/yは「marginを足す前」の位置を
        // 期待するため、ここでmargin分を差し引く。
        let margin_left = super::block::resolve_lpa_or_zero(item_style.margin_left, content_width);
        let margin_top = super::block::resolve_lpa_or_zero(item_style.margin_top, content_width);

        let x = content_x + item_layout.location.x - margin_left;
        let y = content_y + item_layout.location.y - margin_top;

        // flexアイテムは新しいフォーマッティングコンテキストを確立する
        // (`float`はアイテム自身には効果を持たない、CSS仕様通り)ため、
        // アイテムごとに独立した`FloatContext`を使う(`table.rs`のセルと同じ
        // 方針)。
        let mut item_float_ctx = FloatContext::new();
        let laid = layout_box_with_forced_size_ignoring_positioned(
            item,
            styles,
            fonts,
            content_width,
            content_w,
            content_h,
            &mut item_float_ctx,
            x,
            y,
        );
        result.push(laid);
    }

    let root_layout = tree
        .layout(root)
        .expect("直前にcompute_layout_with_measureで計算済み");

    // Gridのページ分割に使う行トラック情報を取り出す。
    let row_tracks = match (mode, tree.detailed_layout_info(root)) {
        (TaffyMode::Grid, tf::DetailedLayoutInfo::Grid(info)) => Some(GridRowTracks {
            sizes: info.rows.sizes.clone(),
            gutters: info.rows.gutters.clone(),
        }),
        _ => None,
    };

    TaffySubtreeLayout {
        items: result,
        container_height: root_layout.size.height,
        row_tracks,
    }
}

fn container_taffy_style(style: &ComputedStyle, content_width: f32) -> tf::Style {
    tf::Style {
        display: tf::Display::Flex,
        flex_direction: map_flex_direction(style.flex_direction),
        flex_wrap: map_flex_wrap(style.flex_wrap),
        justify_content: Some(map_justify_content(style.justify_content)),
        align_items: Some(map_align_items(style.align_items)),
        align_content: Some(map_align_content(style.align_content)),
        gap: tf::Size {
            width: map_length_percentage(style.column_gap),
            height: map_length_percentage(style.row_gap),
        },
        // 高さは明示指定(`height: 100px`等)があればそれをtaffyへ伝える
        // (`align-items`/`align-content`がコンテナの実際の高さを基準に
        // 揃えられるようにするため)。`auto`ならtaffyが内容に基づく自然な
        // 高さを計算する(呼び出し側`block.rs`の`resolve_height`が明示
        // `height`の場合に上書きする、`layout_table`と同じ役割分担)。
        size: tf::Size {
            width: tf::Dimension::length(content_width),
            height: map_dimension(style.height),
        },
        // `min-*`/`max-*`はtaffyへそのまま委譲する。flex文脈ではtaffyが
        // コンテナ基準でパーセンテージを解決できるため、ブロック側と違い
        // 高さのパーセンテージも有効になる(既存の`height`と同じ非対称性)。
        min_size: tf::Size {
            width: map_length_percentage_dimension(style.min_width),
            height: map_length_percentage_dimension(style.min_height),
        },
        max_size: tf::Size {
            width: map_max_size(style.max_width),
            height: map_max_size(style.max_height),
        },
        // `aspect-ratio`もtaffyへ委譲する。
        aspect_ratio: style.aspect_ratio.ratio,
        ..Default::default()
    }
}

pub(super) fn item_taffy_style(style: &ComputedStyle) -> tf::Style {
    let border = resolve_border(style);
    tf::Style {
        size: tf::Size {
            width: map_dimension(style.width),
            height: map_dimension(style.height),
        },
        margin: tf::Rect {
            left: map_margin(style.margin_left),
            right: map_margin(style.margin_right),
            top: map_margin(style.margin_top),
            bottom: map_margin(style.margin_bottom),
        },
        padding: tf::Rect {
            left: map_length_percentage(style.padding_left),
            right: map_length_percentage(style.padding_right),
            top: map_length_percentage(style.padding_top),
            bottom: map_length_percentage(style.padding_bottom),
        },
        border: tf::Rect {
            left: tf::LengthPercentage::length(border.left),
            right: tf::LengthPercentage::length(border.right),
            top: tf::LengthPercentage::length(border.top),
            bottom: tf::LengthPercentage::length(border.bottom),
        },
        min_size: tf::Size {
            width: map_length_percentage_dimension(style.min_width),
            height: map_length_percentage_dimension(style.min_height),
        },
        max_size: tf::Size {
            width: map_max_size(style.max_width),
            height: map_max_size(style.max_height),
        },
        aspect_ratio: style.aspect_ratio.ratio,
        align_self: map_align_self(style.align_self),
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        flex_basis: map_flex_basis(style.flex_basis),
        box_sizing: map_box_sizing(style.box_sizing),
        ..Default::default()
    }
}

fn map_flex_direction(v: FlexDirection) -> tf::FlexDirection {
    match v {
        FlexDirection::Row => tf::FlexDirection::Row,
        FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
        FlexDirection::Column => tf::FlexDirection::Column,
        FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
    }
}

fn map_flex_wrap(v: FlexWrap) -> tf::FlexWrap {
    match v {
        FlexWrap::NoWrap => tf::FlexWrap::NoWrap,
        FlexWrap::Wrap => tf::FlexWrap::Wrap,
        FlexWrap::WrapReverse => tf::FlexWrap::WrapReverse,
    }
}

pub(super) fn map_justify_content(v: JustifyContent) -> tf::JustifyContent {
    match v {
        JustifyContent::FlexStart => tf::JustifyContent::FLEX_START,
        JustifyContent::FlexEnd => tf::JustifyContent::FLEX_END,
        JustifyContent::Center => tf::JustifyContent::CENTER,
        JustifyContent::SpaceBetween => tf::JustifyContent::SPACE_BETWEEN,
        JustifyContent::SpaceAround => tf::JustifyContent::SPACE_AROUND,
        JustifyContent::SpaceEvenly => tf::JustifyContent::SPACE_EVENLY,
    }
}

pub(super) fn map_align_items(v: AlignItems) -> tf::AlignItems {
    match v {
        AlignItems::FlexStart => tf::AlignItems::FLEX_START,
        AlignItems::FlexEnd => tf::AlignItems::FLEX_END,
        AlignItems::Center => tf::AlignItems::CENTER,
        AlignItems::Baseline => tf::AlignItems::BASELINE,
        AlignItems::Stretch => tf::AlignItems::STRETCH,
    }
}

pub(super) fn map_align_content(v: AlignContent) -> tf::AlignContent {
    match v {
        AlignContent::FlexStart => tf::AlignContent::FLEX_START,
        AlignContent::FlexEnd => tf::AlignContent::FLEX_END,
        AlignContent::Center => tf::AlignContent::CENTER,
        AlignContent::Stretch => tf::AlignContent::STRETCH,
        AlignContent::SpaceBetween => tf::AlignContent::SPACE_BETWEEN,
        AlignContent::SpaceAround => tf::AlignContent::SPACE_AROUND,
        AlignContent::SpaceEvenly => tf::AlignContent::SPACE_EVENLY,
    }
}

/// `align-self: auto`(初期値)は`None`にする(taffyが親の`align-items`を使う)。
pub(super) fn map_align_self(v: AlignSelf) -> Option<tf::AlignSelf> {
    match v {
        AlignSelf::Auto => None,
        AlignSelf::FlexStart => Some(tf::AlignSelf::FLEX_START),
        AlignSelf::FlexEnd => Some(tf::AlignSelf::FLEX_END),
        AlignSelf::Center => Some(tf::AlignSelf::CENTER),
        AlignSelf::Baseline => Some(tf::AlignSelf::BASELINE),
        AlignSelf::Stretch => Some(tf::AlignSelf::STRETCH),
    }
}

fn map_box_sizing(v: BoxSizing) -> tf::BoxSizing {
    match v {
        BoxSizing::ContentBox => tf::BoxSizing::ContentBox,
        BoxSizing::BorderBox => tf::BoxSizing::BorderBox,
    }
}

pub(super) fn map_length_percentage(v: LengthPercentage) -> tf::LengthPercentage {
    match v {
        LengthPercentage::Length(px) => tf::LengthPercentage::length(px),
        LengthPercentage::Percentage(p) => tf::LengthPercentage::percent(p),
        // taffyはpx+%の複合を表現できないため、gap等のcalcはpx成分のみ渡す
        // (calcの主用途はflex外のwidth/margin。既知の簡略化)。
        LengthPercentage::Calc { px, .. } => tf::LengthPercentage::length(px),
    }
}

pub(super) fn map_dimension(v: LengthPercentageOrAuto) -> tf::Dimension {
    match v {
        LengthPercentageOrAuto::Auto => tf::Dimension::auto(),
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => {
            tf::Dimension::length(px)
        }
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Percentage(p)) => {
            tf::Dimension::percent(p)
        }
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc { px, .. }) => {
            tf::Dimension::length(px)
        }
    }
}

/// `min-width`/`min-height`(初期値`0`)をtaffyの`Dimension`へ。
pub(super) fn map_length_percentage_dimension(v: LengthPercentage) -> tf::Dimension {
    match v {
        LengthPercentage::Length(px) => tf::Dimension::length(px),
        LengthPercentage::Percentage(p) => tf::Dimension::percent(p),
        // taffyはpx+%の複合を表現できないためpx成分のみ渡す
        // (`map_length_percentage`と同じ簡略化)。
        LengthPercentage::Calc { px, .. } => tf::Dimension::length(px),
    }
}

/// `max-width`/`max-height`をtaffyの`Dimension`へ。`none`は`auto`(上限なし)。
pub(super) fn map_max_size(v: MaxSize) -> tf::Dimension {
    match v {
        MaxSize::None => tf::Dimension::auto(),
        MaxSize::LengthPercentage(lp) => map_length_percentage_dimension(lp),
    }
}

fn map_margin(v: LengthPercentageOrAuto) -> tf::LengthPercentageAuto {
    match v {
        LengthPercentageOrAuto::Auto => tf::LengthPercentageAuto::auto(),
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => {
            tf::LengthPercentageAuto::length(px)
        }
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Percentage(p)) => {
            tf::LengthPercentageAuto::percent(p)
        }
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc { px, .. }) => {
            tf::LengthPercentageAuto::length(px)
        }
    }
}

fn map_flex_basis(v: FlexBasis) -> tf::Dimension {
    match v {
        FlexBasis::Auto => tf::Dimension::auto(),
        FlexBasis::LengthPercentage(LengthPercentage::Length(px)) => tf::Dimension::length(px),
        FlexBasis::LengthPercentage(LengthPercentage::Percentage(p)) => tf::Dimension::percent(p),
        FlexBasis::LengthPercentage(LengthPercentage::Calc { px, .. }) => tf::Dimension::length(px),
    }
}
