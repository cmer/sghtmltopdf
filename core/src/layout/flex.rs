//! Flexbox(`display: flex`)を、既存box treeのサブツリーとしてtaffyへ
//! ブリッジする。設計判断は[0034](../../../docs/decisions/0034-flexbox-design.md)
//! に記録済み。
//!
//! taffyは自前でノードツリー(`TaffyTree`)を持つ設計のため、flexコンテナ・
//! 各アイテムに対応するtaffyのリーフノードを都度組み立て、`compute_layout_with_measure`
//! で1回だけレイアウトを計算する。テキスト等の内在サイズが必要なリーフには
//! 採寸(measure)コールバックを渡し、その中で既存のブロック/インライン/テーブル
//! レイアウト関数を呼んで実測する(決定2)。計算結果(各アイテムの確定した
//! 位置・サイズ)は、`layout_box_with_forced_size`でもう一度実際のレイアウトを
//! 行うことで`LaidOutBox`へ変換する(2パス方式、決定2)。
//!
//! taffyの型は自前の同名CSS型(`crate::style::FlexDirection`等)と衝突するため
//! `tf`という別名で参照する。

use std::collections::HashMap;

use taffy as tf;

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::style::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, ComputedStyle, FlexBasis, FlexDirection,
    FlexWrap, JustifyContent, LengthPercentage, LengthPercentageOrAuto,
};

use super::block::{
    box_style, layout_box_with_forced_size_ignoring_positioned,
    layout_box_with_forced_width_ignoring_positioned, resolve_border, resolve_padding, LaidOutBox,
};
use super::box_tree::FlexBox;
use super::float_ctx::FloatContext;
use super::table::measure_natural_content_width;

/// flexコンテナのcontent box内(`content_x`/`content_y`起点、幅`content_width`)
/// でflexアイテム群をレイアウトする。返り値はレイアウト済みの各アイテムと、
/// コンテナの自然な(内容に基づく)content-box高さ(呼び出し側`block.rs`が
/// 明示`height`指定で上書きする前の値、`layout_table`と同じ役割分担)。
pub(super) fn layout_flex(
    flex: &FlexBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    container_style: &ComputedStyle,
    content_width: f32,
    content_x: f32,
    content_y: f32,
) -> (Vec<LaidOutBox>, f32) {
    if flex.items.is_empty() {
        return (Vec::new(), 0.0);
    }

    let mut tree: tf::TaffyTree<usize> = tf::TaffyTree::new();

    let leaves: Vec<tf::NodeId> = flex
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let item_style = box_style(item, styles);
            tree.new_leaf_with_context(item_taffy_style(&item_style), index)
                .expect("taffyへのリーフノード追加は失敗しない")
        })
        .collect();

    let root_style = container_taffy_style(container_style, content_width);
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
            let item = &flex.items[index];
            let item_style = box_style(item, styles);

            // measureが返すのはcontent-box基準のサイズ(taffy自身がpadding/borderを
            // 加算する、`compute::leaf::compute_leaf_layout`で実測済みの規約)。
            let width = known_dimensions.width.unwrap_or_else(|| {
                match available_space.width {
                    tf::AvailableSpace::Definite(w) => w,
                    // min-contentとmax-contentは区別しない(決定2、既知の簡略化)。
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

    let mut result = Vec::with_capacity(flex.items.len());
    for (index, item) in flex.items.iter().enumerate() {
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
    (result, root_layout.size.height)
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
        ..Default::default()
    }
}

fn item_taffy_style(style: &ComputedStyle) -> tf::Style {
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

fn map_justify_content(v: JustifyContent) -> tf::JustifyContent {
    match v {
        JustifyContent::FlexStart => tf::JustifyContent::FLEX_START,
        JustifyContent::FlexEnd => tf::JustifyContent::FLEX_END,
        JustifyContent::Center => tf::JustifyContent::CENTER,
        JustifyContent::SpaceBetween => tf::JustifyContent::SPACE_BETWEEN,
        JustifyContent::SpaceAround => tf::JustifyContent::SPACE_AROUND,
        JustifyContent::SpaceEvenly => tf::JustifyContent::SPACE_EVENLY,
    }
}

fn map_align_items(v: AlignItems) -> tf::AlignItems {
    match v {
        AlignItems::FlexStart => tf::AlignItems::FLEX_START,
        AlignItems::FlexEnd => tf::AlignItems::FLEX_END,
        AlignItems::Center => tf::AlignItems::CENTER,
        AlignItems::Baseline => tf::AlignItems::BASELINE,
        AlignItems::Stretch => tf::AlignItems::STRETCH,
    }
}

fn map_align_content(v: AlignContent) -> tf::AlignContent {
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
fn map_align_self(v: AlignSelf) -> Option<tf::AlignSelf> {
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

fn map_length_percentage(v: LengthPercentage) -> tf::LengthPercentage {
    match v {
        LengthPercentage::Length(px) => tf::LengthPercentage::length(px),
        LengthPercentage::Percentage(p) => tf::LengthPercentage::percent(p),
        // taffyはpx+%の複合を表現できないため、gap等のcalcはpx成分のみ渡す
        // (calcの主用途はflex外のwidth/margin。既知の簡略化)。
        LengthPercentage::Calc { px, .. } => tf::LengthPercentage::length(px),
    }
}

fn map_dimension(v: LengthPercentageOrAuto) -> tf::Dimension {
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
