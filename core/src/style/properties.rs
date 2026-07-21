//! `Declaration: value;`のパースと、プロパティ宣言の型。

use cssparser::{match_ignore_ascii_case, CowRcStr, ParseError, Parser, Token};

use super::values::{
    BackgroundAttachment, BackgroundRepeat, BorderCollapse, BorderStyle, BreakBetween, BreakInside,
    CaptionSide, Clear, Color, ContentPart, Display, EmptyCells, Float, FontStyle, FontWeight,
    ListStylePosition, ListStyleType, Overflow, Position, QuotePair, SpecifiedBackgroundPosition,
    SpecifiedBackgroundSize, SpecifiedCornerRadius, SpecifiedLength, SpecifiedLengthPercentage,
    SpecifiedLengthPercentageOrAuto, SpecifiedLineHeight, SpecifiedSpacing, TableLayout, TextAlign,
    TextDecorationLine, TextTransform, VerticalAlign, Visibility, WhiteSpace, ZIndex,
};

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyDeclaration {
    Display(Display),
    Width(SpecifiedLengthPercentageOrAuto),
    Height(SpecifiedLengthPercentageOrAuto),
    MarginTop(SpecifiedLengthPercentageOrAuto),
    MarginRight(SpecifiedLengthPercentageOrAuto),
    MarginBottom(SpecifiedLengthPercentageOrAuto),
    MarginLeft(SpecifiedLengthPercentageOrAuto),
    PaddingTop(SpecifiedLengthPercentage),
    PaddingRight(SpecifiedLengthPercentage),
    PaddingBottom(SpecifiedLengthPercentage),
    PaddingLeft(SpecifiedLengthPercentage),
    BorderTopWidth(SpecifiedLength),
    BorderRightWidth(SpecifiedLength),
    BorderBottomWidth(SpecifiedLength),
    BorderLeftWidth(SpecifiedLength),
    BorderTopColor(Color),
    BorderRightColor(Color),
    BorderBottomColor(Color),
    BorderLeftColor(Color),
    BorderTopStyle(BorderStyle),
    BorderRightStyle(BorderStyle),
    BorderBottomStyle(BorderStyle),
    BorderLeftStyle(BorderStyle),
    BorderTopLeftRadius(SpecifiedCornerRadius),
    BorderTopRightRadius(SpecifiedCornerRadius),
    BorderBottomRightRadius(SpecifiedCornerRadius),
    BorderBottomLeftRadius(SpecifiedCornerRadius),
    FontSize(SpecifiedLength),
    FontFamily(Vec<String>),
    FontWeight(FontWeight),
    FontStyle(FontStyle),
    Color(Color),
    BackgroundColor(Color),
    /// `url(...)`(生の値、解決は呼び出し側任せ、`FontFaceSource::Url`と
    /// 同じ方針)。`None`は`none`(背景画像なし)を表す。
    BackgroundImage(Option<String>),
    BackgroundPosition(SpecifiedBackgroundPosition),
    BackgroundSize(SpecifiedBackgroundSize),
    BackgroundRepeat(BackgroundRepeat),
    /// `fixed`は`scroll`と同一視して描画する([0025](
    /// ../../../docs/decisions/0025-background-details-design.md)決定5)。
    BackgroundAttachment(BackgroundAttachment),
    TextDecorationLine(TextDecorationLine),
    /// `::before`/`::after`/`::first-letter`用の`content`。`None`は`none`/
    /// `normal`(生成ボックスなし)。文字列リテラル・`attr()`・`counter()`/
    /// `counters()`・引用符キーワードの連結に対応する([0024](
    /// ../../../docs/decisions/0024-generated-content-design.md)決定1)。
    Content(Option<Vec<ContentPart>>),
    BreakBefore(BreakBetween),
    BreakAfter(BreakBetween),
    BreakInside(BreakInside),
    Orphans(u32),
    Widows(u32),
    Float(Float),
    Clear(Clear),
    /// `absolute`/`fixed`は非対応
    /// ([0018](../../../docs/decisions/0018-css21-css3-coverage-strategy.md))。
    /// `parse_position`がこれらのキーワードをパースエラーとして拒否するため、
    /// このバリアントは常に`Static`/`Relative`のいずれかを持つ。
    Position(Position),
    Top(SpecifiedLengthPercentageOrAuto),
    Right(SpecifiedLengthPercentageOrAuto),
    Bottom(SpecifiedLengthPercentageOrAuto),
    Left(SpecifiedLengthPercentageOrAuto),
    TextAlign(TextAlign),
    LineHeight(SpecifiedLineHeight),
    TextIndent(SpecifiedLengthPercentage),
    WhiteSpace(WhiteSpace),
    LetterSpacing(SpecifiedSpacing),
    WordSpacing(SpecifiedSpacing),
    TextTransform(TextTransform),
    BorderCollapse(BorderCollapse),
    /// `border-spacing`(水平, 垂直)。1値指定は両方に同じ値を使う(仕様通り)。
    BorderSpacing(SpecifiedLength, SpecifiedLength),
    CaptionSide(CaptionSide),
    TableLayout(TableLayout),
    EmptyCells(EmptyCells),
    VerticalAlign(VerticalAlign),
    ListStyleType(ListStyleType),
    ListStylePosition(ListStylePosition),
    /// `url(...)`(生の値、解決は呼び出し側任せ)。`None`は`none`。
    /// 実際には常に`list-style-type`のテキストマーカーへフォールバックし、
    /// 画像マーカー自体は描画しない([0022](
    /// ../../../docs/decisions/0022-list-style-design.md)決定5)。
    ListStyleImage(Option<String>),
    /// `hidden`/`scroll`/`auto`は全て同じクリップ処理として扱う([0023](
    /// ../../../docs/decisions/0023-box-model-details-design.md)決定1)。
    Overflow(Overflow),
    /// `position: static`の要素には効果を持たない(仕様通り、決定2)。
    ZIndex(ZIndex),
    /// `collapse`は`hidden`と同一視する(決定4)。
    Visibility(Visibility),
    OutlineWidth(SpecifiedLength),
    /// `outline-style`。`auto`(UA依存の既定フォーカスリング)は非対応、
    /// `border-style`と同じ値集合+`none`のみ受け付ける(決定3)。
    OutlineStyle(BorderStyle),
    OutlineColor(Color),
    /// `counter-reset: name [value]`(複数併記可)。空のVecは`none`。[0024]決定2。
    CounterReset(Vec<(String, i32)>),
    /// `counter-increment: name [value]`(複数併記可、値省略時は1)。
    CounterIncrement(Vec<(String, i32)>),
    /// `quotes`。`None`は`none`(常に空文字列を生成する、決定3)。
    Quotes(Option<Vec<QuotePair>>),
}

/// プロパティ名から値をパースする。ショートハンド(`margin`/`padding`/`border`)は
/// 対応するロングハンド宣言に展開して返す。
pub fn parse_declaration<'i>(
    name: &CowRcStr<'i>,
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;

    match_ignore_ascii_case! { name,
        "display" => Ok(vec![D::Display(parse_display(input)?)]),
        "width" => Ok(vec![D::Width(parse_length_percentage_or_auto(input)?)]),
        "height" => Ok(vec![D::Height(parse_length_percentage_or_auto(input)?)]),
        "margin" => parse_margin_shorthand(input),
        "margin-top" => Ok(vec![D::MarginTop(parse_length_percentage_or_auto(input)?)]),
        "margin-right" => Ok(vec![D::MarginRight(parse_length_percentage_or_auto(input)?)]),
        "margin-bottom" => Ok(vec![D::MarginBottom(parse_length_percentage_or_auto(input)?)]),
        "margin-left" => Ok(vec![D::MarginLeft(parse_length_percentage_or_auto(input)?)]),
        "padding" => parse_padding_shorthand(input),
        "padding-top" => Ok(vec![D::PaddingTop(parse_length_percentage(input)?)]),
        "padding-right" => Ok(vec![D::PaddingRight(parse_length_percentage(input)?)]),
        "padding-bottom" => Ok(vec![D::PaddingBottom(parse_length_percentage(input)?)]),
        "padding-left" => Ok(vec![D::PaddingLeft(parse_length_percentage(input)?)]),
        "border" => parse_border_shorthand(input),
        "border-width" => parse_border_width_shorthand(input),
        "border-color" => parse_border_color_shorthand(input),
        "border-style" => parse_border_style_shorthand(input),
        "border-radius" => parse_border_radius_shorthand(input),
        "border-top-left-radius" => Ok(vec![D::BorderTopLeftRadius(parse_corner_radius(input)?)]),
        "border-top-right-radius" => Ok(vec![D::BorderTopRightRadius(parse_corner_radius(input)?)]),
        "border-bottom-right-radius" => {
            Ok(vec![D::BorderBottomRightRadius(parse_corner_radius(input)?)])
        },
        "border-bottom-left-radius" => {
            Ok(vec![D::BorderBottomLeftRadius(parse_corner_radius(input)?)])
        },
        "font-size" => Ok(vec![D::FontSize(parse_length(input)?)]),
        "font-family" => Ok(vec![D::FontFamily(parse_font_family(input)?)]),
        "font-weight" => Ok(vec![D::FontWeight(parse_font_weight(input)?)]),
        "font-style" => Ok(vec![D::FontStyle(parse_font_style(input)?)]),
        "color" => Ok(vec![D::Color(parse_color(input)?)]),
        "background-color" => Ok(vec![D::BackgroundColor(parse_color(input)?)]),
        "background-image" => Ok(vec![D::BackgroundImage(parse_background_image(input)?)]),
        "background-position" => {
            Ok(vec![D::BackgroundPosition(parse_background_position(input)?)])
        },
        "background-size" => Ok(vec![D::BackgroundSize(parse_background_size(input)?)]),
        "background-repeat" => Ok(vec![D::BackgroundRepeat(parse_background_repeat(input)?)]),
        "background-attachment" => {
            Ok(vec![D::BackgroundAttachment(parse_background_attachment(input)?)])
        },
        "background" => parse_background_shorthand(input),
        "text-decoration" | "text-decoration-line" => {
            Ok(vec![D::TextDecorationLine(parse_text_decoration_line(input)?)])
        },
        "content" => Ok(vec![D::Content(parse_content(input)?)]),
        // `page-break-*`は旧世代のプロパティ名(wkhtmltopdf/wicked_pdf資産からの
        // 移行コストを下げるため、`break-*`のエイリアスとして受理する)。
        "break-before" | "page-break-before" => {
            Ok(vec![D::BreakBefore(parse_break_between(input)?)])
        },
        "break-after" | "page-break-after" => {
            Ok(vec![D::BreakAfter(parse_break_between(input)?)])
        },
        "break-inside" | "page-break-inside" => {
            Ok(vec![D::BreakInside(parse_break_inside(input)?)])
        },
        "orphans" => Ok(vec![D::Orphans(parse_positive_integer(input)?)]),
        "widows" => Ok(vec![D::Widows(parse_positive_integer(input)?)]),
        "float" => Ok(vec![D::Float(parse_float(input)?)]),
        "clear" => Ok(vec![D::Clear(parse_clear(input)?)]),
        "position" => Ok(vec![D::Position(parse_position(input)?)]),
        "top" => Ok(vec![D::Top(parse_length_percentage_or_auto(input)?)]),
        "right" => Ok(vec![D::Right(parse_length_percentage_or_auto(input)?)]),
        "bottom" => Ok(vec![D::Bottom(parse_length_percentage_or_auto(input)?)]),
        "left" => Ok(vec![D::Left(parse_length_percentage_or_auto(input)?)]),
        "text-align" => Ok(vec![D::TextAlign(parse_text_align(input)?)]),
        "line-height" => Ok(vec![D::LineHeight(parse_line_height(input)?)]),
        "text-indent" => Ok(vec![D::TextIndent(parse_length_percentage(input)?)]),
        "white-space" => Ok(vec![D::WhiteSpace(parse_white_space(input)?)]),
        "letter-spacing" => Ok(vec![D::LetterSpacing(parse_spacing(input)?)]),
        "word-spacing" => Ok(vec![D::WordSpacing(parse_spacing(input)?)]),
        "text-transform" => Ok(vec![D::TextTransform(parse_text_transform(input)?)]),
        "border-collapse" => Ok(vec![D::BorderCollapse(parse_border_collapse(input)?)]),
        "border-spacing" => {
            let (h, v) = parse_border_spacing(input)?;
            Ok(vec![D::BorderSpacing(h, v)])
        },
        "caption-side" => Ok(vec![D::CaptionSide(parse_caption_side(input)?)]),
        "table-layout" => Ok(vec![D::TableLayout(parse_table_layout(input)?)]),
        "empty-cells" => Ok(vec![D::EmptyCells(parse_empty_cells(input)?)]),
        "vertical-align" => Ok(vec![D::VerticalAlign(parse_vertical_align(input)?)]),
        "list-style-type" => Ok(vec![D::ListStyleType(parse_list_style_type(input)?)]),
        "list-style-position" => {
            Ok(vec![D::ListStylePosition(parse_list_style_position(input)?)])
        },
        "list-style-image" => Ok(vec![D::ListStyleImage(parse_list_style_image(input)?)]),
        "list-style" => parse_list_style_shorthand(input),
        "overflow" => Ok(vec![D::Overflow(parse_overflow(input)?)]),
        "z-index" => Ok(vec![D::ZIndex(parse_z_index(input)?)]),
        "visibility" => Ok(vec![D::Visibility(parse_visibility(input)?)]),
        "outline-width" => Ok(vec![D::OutlineWidth(parse_length(input)?)]),
        "outline-style" => Ok(vec![D::OutlineStyle(parse_border_style_keyword(input)?)]),
        "outline-color" => Ok(vec![D::OutlineColor(parse_color(input)?)]),
        "outline" => parse_outline_shorthand(input),
        "counter-reset" => Ok(vec![D::CounterReset(parse_counter_list(input, 0)?)]),
        "counter-increment" => Ok(vec![D::CounterIncrement(parse_counter_list(input, 1)?)]),
        "quotes" => Ok(vec![D::Quotes(parse_quotes(input)?)]),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_margin_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top, right, bottom, left) = parse_four_sides(input, parse_length_percentage_or_auto)?;
    Ok(vec![
        D::MarginTop(top),
        D::MarginRight(right),
        D::MarginBottom(bottom),
        D::MarginLeft(left),
    ])
}

fn parse_padding_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top, right, bottom, left) = parse_four_sides(input, parse_length_percentage)?;
    Ok(vec![
        D::PaddingTop(top),
        D::PaddingRight(right),
        D::PaddingBottom(bottom),
        D::PaddingLeft(left),
    ])
}

/// CSSの1〜4値ショートハンド展開規則(上/右/下/左)。
fn parse_four_sides<'i, T: Copy>(
    input: &mut Parser<'i, '_>,
    mut parse_one: impl FnMut(&mut Parser<'i, '_>) -> Result<T, ParseError<'i, ()>>,
) -> Result<(T, T, T, T), ParseError<'i, ()>> {
    let top = parse_one(input)?;
    let Ok(right) = input.try_parse(&mut parse_one) else {
        return Ok((top, top, top, top));
    };
    let Ok(bottom) = input.try_parse(&mut parse_one) else {
        return Ok((top, right, top, right));
    };
    let Ok(left) = input.try_parse(&mut parse_one) else {
        return Ok((top, right, bottom, right));
    };
    Ok((top, right, bottom, left))
}

/// `border`ショートハンドの簡易実装。`border-width`/`border-style`/`border-color`を
/// 同時に指定でき、指定順序は問わない(CSSの`border`ショートハンドの仕様通り)。
/// いずれも4辺に同じ値を適用する(`border-top`等の辺別ショートハンドは非対応)。
/// `border-color`省略時は宣言を生成しない(計算スタイル側で初期値`currentcolor`
/// として扱う)。
fn parse_border_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let mut width = SpecifiedLength::Px(0.0);
    let mut style = BorderStyle::None;
    let mut color = None;

    loop {
        if let Ok(w) = input.try_parse(parse_length) {
            width = w;
            continue;
        }
        if let Ok(s) = input.try_parse(parse_border_style_keyword) {
            style = s;
            continue;
        }
        if let Ok(c) = input.try_parse(parse_color) {
            color = Some(c);
            continue;
        }
        break;
    }

    let mut decls = vec![
        D::BorderTopWidth(width),
        D::BorderRightWidth(width),
        D::BorderBottomWidth(width),
        D::BorderLeftWidth(width),
        D::BorderTopStyle(style),
        D::BorderRightStyle(style),
        D::BorderBottomStyle(style),
        D::BorderLeftStyle(style),
    ];
    if let Some(c) = color {
        decls.extend([
            D::BorderTopColor(c),
            D::BorderRightColor(c),
            D::BorderBottomColor(c),
            D::BorderLeftColor(c),
        ]);
    }
    Ok(decls)
}

fn parse_border_width_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top, right, bottom, left) = parse_four_sides(input, parse_length)?;
    Ok(vec![
        D::BorderTopWidth(top),
        D::BorderRightWidth(right),
        D::BorderBottomWidth(bottom),
        D::BorderLeftWidth(left),
    ])
}

fn parse_border_color_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top, right, bottom, left) = parse_four_sides(input, parse_color)?;
    Ok(vec![
        D::BorderTopColor(top),
        D::BorderRightColor(right),
        D::BorderBottomColor(bottom),
        D::BorderLeftColor(left),
    ])
}

fn parse_border_style_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top, right, bottom, left) = parse_four_sides(input, parse_border_style_keyword)?;
    Ok(vec![
        D::BorderTopStyle(top),
        D::BorderRightStyle(right),
        D::BorderBottomStyle(bottom),
        D::BorderLeftStyle(left),
    ])
}

/// `border-radius`ショートハンドの簡易実装。CSSの角丸半径は4値展開でも
/// 「上→右→下→左」ではなく「左上→右上→右下→左下」の順序だが、
/// `parse_four_sides`は値の個数に応じた展開規則(1〜4値)のみを担う汎用ヘルパーで
/// 各スロットの意味には関与しないため、そのまま再利用できる。
/// 楕円形(`/`区切りの水平・垂直別半径)は非対応(常に真円)。
/// `border-radius`ショートハンドの簡易実装。CSSの角丸半径は4値展開でも
/// 「上→右→下→左」ではなく「左上→右上→右下→左下」の順序だが、
/// `parse_four_sides`は値の個数に応じた展開規則(1〜4値)のみを担う汎用ヘルパーで
/// 各スロットの意味には関与しないため、そのまま再利用できる。`/`区切りで
/// 水平・垂直の半径を別々に指定する楕円構文に対応する([0023](
/// ../../../docs/decisions/0023-box-model-details-design.md)決定6)。
fn parse_border_radius_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (h_tl, h_tr, h_br, h_bl) = parse_four_sides(input, parse_length)?;
    let vertical = if input.try_parse(|input| input.expect_delim('/')).is_ok() {
        Some(parse_four_sides(input, parse_length)?)
    } else {
        None
    };
    let (v_tl, v_tr, v_br, v_bl) = vertical.unwrap_or((h_tl, h_tr, h_br, h_bl));

    Ok(vec![
        D::BorderTopLeftRadius(SpecifiedCornerRadius {
            horizontal: h_tl,
            vertical: v_tl,
        }),
        D::BorderTopRightRadius(SpecifiedCornerRadius {
            horizontal: h_tr,
            vertical: v_tr,
        }),
        D::BorderBottomRightRadius(SpecifiedCornerRadius {
            horizontal: h_br,
            vertical: v_br,
        }),
        D::BorderBottomLeftRadius(SpecifiedCornerRadius {
            horizontal: h_bl,
            vertical: v_bl,
        }),
    ])
}

/// `border-top-left-radius`等のロングハンド。`<length>{1,2}`(水平, 垂直の順、
/// 省略時は水平と同じ=真円)を受け付ける([0021]の`border-spacing`と同じパターン)。
fn parse_corner_radius<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedCornerRadius, ParseError<'i, ()>> {
    let horizontal = parse_length(input)?;
    let vertical = input.try_parse(parse_length).unwrap_or(horizontal);
    Ok(SpecifiedCornerRadius {
        horizontal,
        vertical,
    })
}

/// `border-style`/`outline-style`共通のキーワード。`groove`/`ridge`/`inset`/
/// `outset`(border-colorから2階調の疑似立体陰影を算出する)は[0023]決定5で対応
/// (既存の非対応方針から変更、ユーザー確認済み)。
fn parse_border_style_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderStyle, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "none" | "hidden" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "groove" => BorderStyle::Groove,
        "ridge" => BorderStyle::Ridge,
        "inset" => BorderStyle::Inset,
        "outset" => BorderStyle::Outset,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `overflow`。`hidden`/`scroll`/`auto`は全て同じクリップ処理として扱う
/// ([0023]決定1)。
fn parse_overflow<'i>(input: &mut Parser<'i, '_>) -> Result<Overflow, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "visible" => Overflow::Visible,
        "hidden" => Overflow::Hidden,
        "scroll" => Overflow::Scroll,
        "auto" => Overflow::Auto,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `z-index`。`auto | <integer>`。
fn parse_z_index<'i>(input: &mut Parser<'i, '_>) -> Result<ZIndex, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(ZIndex::Auto);
    }
    Ok(ZIndex::Value(input.expect_integer()?))
}

/// `visibility`。`collapse`は`hidden`と同一視する([0023]決定4)。
fn parse_visibility<'i>(input: &mut Parser<'i, '_>) -> Result<Visibility, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "visible" => Visibility::Visible,
        "hidden" => Visibility::Hidden,
        "collapse" => Visibility::Collapse,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `outline`ショートハンドの簡易実装。`outline-width`/`outline-style`/
/// `outline-color`を任意の順序・任意の省略で受け付ける(`border`ショートハンドと
/// 同じパターン)。`outline-offset`は非対応、常に0固定([0023]決定3)。
fn parse_outline_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let mut width = SpecifiedLength::Px(0.0);
    let mut style = BorderStyle::None;
    let mut color = None;

    loop {
        if let Ok(w) = input.try_parse(parse_length) {
            width = w;
            continue;
        }
        if let Ok(s) = input.try_parse(parse_border_style_keyword) {
            style = s;
            continue;
        }
        if let Ok(c) = input.try_parse(parse_color) {
            color = Some(c);
            continue;
        }
        break;
    }

    let mut decls = vec![D::OutlineWidth(width), D::OutlineStyle(style)];
    if let Some(c) = color {
        decls.push(D::OutlineColor(c));
    }
    Ok(decls)
}

fn parse_text_align<'i>(input: &mut Parser<'i, '_>) -> Result<TextAlign, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        "center" => TextAlign::Center,
        "justify" => TextAlign::Justify,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_white_space<'i>(input: &mut Parser<'i, '_>) -> Result<WhiteSpace, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "normal" => WhiteSpace::Normal,
        "nowrap" => WhiteSpace::Nowrap,
        "pre" => WhiteSpace::Pre,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_text_transform<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TextTransform, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "none" => TextTransform::None,
        "uppercase" => TextTransform::Uppercase,
        "lowercase" => TextTransform::Lowercase,
        "capitalize" => TextTransform::Capitalize,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `line-height`。`normal | <number> | <length> | <percentage>`。
fn parse_line_height<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedLineHeight, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("normal"))
        .is_ok()
    {
        return Ok(SpecifiedLineHeight::Normal);
    }
    let token = input.next()?.clone();
    match token {
        Token::Number { value, .. } => Ok(SpecifiedLineHeight::Number(value)),
        Token::Percentage { unit_value, .. } => Ok(SpecifiedLineHeight::Percentage(unit_value)),
        Token::Dimension {
            value, ref unit, ..
        } => Ok(SpecifiedLineHeight::Length(parse_length_unit(
            input, value, unit,
        )?)),
        _ => Err(input.new_custom_error(())),
    }
}

/// `letter-spacing`/`word-spacing`共通。`normal | <length>`。
fn parse_spacing<'i>(input: &mut Parser<'i, '_>) -> Result<SpecifiedSpacing, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("normal"))
        .is_ok()
    {
        return Ok(SpecifiedSpacing::Normal);
    }
    Ok(SpecifiedSpacing::Length(parse_length(input)?))
}

fn parse_border_collapse<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderCollapse, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "separate" => BorderCollapse::Separate,
        "collapse" => BorderCollapse::Collapse,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `border-spacing`。`<length>`(水平・垂直に同じ値)または`<length> <length>`
/// (水平, 垂直)。
fn parse_border_spacing<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(SpecifiedLength, SpecifiedLength), ParseError<'i, ()>> {
    let horizontal = parse_length(input)?;
    let vertical = input.try_parse(parse_length).unwrap_or(horizontal);
    Ok((horizontal, vertical))
}

fn parse_caption_side<'i>(input: &mut Parser<'i, '_>) -> Result<CaptionSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "top" => CaptionSide::Top,
        "bottom" => CaptionSide::Bottom,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_table_layout<'i>(input: &mut Parser<'i, '_>) -> Result<TableLayout, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "auto" => TableLayout::Auto,
        "fixed" => TableLayout::Fixed,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_empty_cells<'i>(input: &mut Parser<'i, '_>) -> Result<EmptyCells, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "show" => EmptyCells::Show,
        "hide" => EmptyCells::Hide,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `vertical-align`。テーブルセル文脈専用と割り切る([0021]決定4)。
/// `sub`/`super`/`text-top`/`text-bottom`/`<length>`/`<percentage>`
/// (インライン文脈向けの値)は非対応。
fn parse_vertical_align<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<VerticalAlign, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "top" => VerticalAlign::Top,
        "middle" => VerticalAlign::Middle,
        "bottom" => VerticalAlign::Bottom,
        "baseline" => VerticalAlign::Baseline,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_display<'i>(input: &mut Parser<'i, '_>) -> Result<Display, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "block" => Display::Block,
        "inline" => Display::Inline,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-caption" => Display::TableCaption,
        "list-item" => Display::ListItem,
        "none" => Display::None,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_list_style_type<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ListStyleType, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "disc" => ListStyleType::Disc,
        "circle" => ListStyleType::Circle,
        "square" => ListStyleType::Square,
        "decimal" => ListStyleType::Decimal,
        "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
        "lower-roman" => ListStyleType::LowerRoman,
        "upper-roman" => ListStyleType::UpperRoman,
        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
        "none" => ListStyleType::None,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_list_style_position<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ListStylePosition, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "outside" => ListStylePosition::Outside,
        "inside" => ListStylePosition::Inside,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `list-style-image`。`background-image`と同じ`url(...) | none`の形。
/// 実際には常に`list-style-type`へフォールバックし描画には使わない
/// ([0022](../../../docs/decisions/0022-list-style-design.md)決定5)。
fn parse_list_style_image<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Option<String>, ParseError<'i, ()>> {
    parse_background_image(input)
}

/// `list-style`ショートハンドの簡易実装。`list-style-type`/
/// `list-style-position`/`list-style-image`を任意の順序・任意の省略で受け付ける
/// (`border`ショートハンドと同じパターン、[0022]決定6)。`none`は`list-style-type`/
/// `list-style-image`のどちらの値としても妥当なため、まだ確定していない方の
/// スロットから順に埋める(`type`未確定なら`type: none`、確定済みなら
/// `image: none`)。これにより`list-style: square none`(type=square,
/// image=none)と`list-style: none`(両方none)のどちらも正しく解決できる。
fn parse_list_style_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let mut ty = None;
    let mut position = None;
    let mut image = None;

    loop {
        if position.is_none() {
            if let Ok(p) = input.try_parse(parse_list_style_position) {
                position = Some(p);
                continue;
            }
        }
        if ty.is_none() {
            if let Ok(t) = input.try_parse(parse_list_style_type) {
                ty = Some(t);
                continue;
            }
        }
        if image.is_none() {
            if let Ok(img) = input.try_parse(parse_list_style_image) {
                image = Some(img);
                continue;
            }
        }
        break;
    }

    let mut decls = Vec::new();
    if let Some(p) = position {
        decls.push(D::ListStylePosition(p));
    }
    decls.push(D::ListStyleType(ty.unwrap_or_default()));
    decls.push(D::ListStyleImage(image.unwrap_or(None)));
    Ok(decls)
}

fn parse_float<'i>(input: &mut Parser<'i, '_>) -> Result<Float, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "none" => Float::None,
        "left" => Float::Left,
        "right" => Float::Right,
        _ => return Err(input.new_custom_error(())),
    })
}

fn parse_clear<'i>(input: &mut Parser<'i, '_>) -> Result<Clear, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "none" => Clear::None,
        "left" => Clear::Left,
        "right" => Clear::Right,
        "both" => Clear::Both,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `position`。`absolute`/`fixed`は既知の未対応キーワードとしてパースエラーに
/// する(`border-style`のgroove/ridge等と同じパターン。宣言ごと無視され、他の
/// 宣言には影響しない)。
fn parse_position<'i>(input: &mut Parser<'i, '_>) -> Result<Position, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "static" => Position::Static,
        "relative" => Position::Relative,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `break-before`/`break-after`(および`page-break-before`/`-after`エイリアス)の値。
/// `left`/`right`/`recto`/`verso`(見開き制御)は単一ページサイズ前提のため非対応。
fn parse_break_between<'i>(input: &mut Parser<'i, '_>) -> Result<BreakBetween, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "auto" => BreakBetween::Auto,
        "avoid" | "avoid-page" | "avoid-column" => BreakBetween::Avoid,
        "always" | "page" => BreakBetween::Always,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `break-inside`(および`page-break-inside`エイリアス)の値。
fn parse_break_inside<'i>(input: &mut Parser<'i, '_>) -> Result<BreakInside, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "auto" => BreakInside::Auto,
        "avoid" | "avoid-page" | "avoid-column" => BreakInside::Avoid,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `orphans`/`widows`の値。0以下は無効(仕様上も1以上の整数のみ有効)。
fn parse_positive_integer<'i>(input: &mut Parser<'i, '_>) -> Result<u32, ParseError<'i, ()>> {
    let value = input.expect_integer()?;
    if value < 1 {
        return Err(input.new_custom_error(()));
    }
    Ok(value as u32)
}

/// キーワード(`normal`/`bold`)と数値(`100`〜`900`)のどちらも受け付ける。
/// 数値は600以上を`Bold`とみなす簡略実装(実際の太字フォントを持たず、
/// 描画時に疑似太字で表現するため、細かい太さの段階は区別しない)。
pub(crate) fn parse_font_weight<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FontWeight, ParseError<'i, ()>> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return Ok(match_ignore_ascii_case! { &ident,
            "normal" => FontWeight::Normal,
            "bold" => FontWeight::Bold,
            _ => return Err(input.new_custom_error(())),
        });
    }
    let value = input.expect_number()?;
    Ok(if value >= 600.0 {
        FontWeight::Bold
    } else {
        FontWeight::Normal
    })
}

pub(crate) fn parse_font_style<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FontStyle, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "normal" => FontStyle::Normal,
        // `oblique`は専用の傾斜角を持たないため`italic`と同一視する。
        "italic" | "oblique" => FontStyle::Italic,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `text-decoration`/`text-decoration-line`の簡易実装。`underline`/`line-through`は
/// 併記可能(`underline line-through`)。`overline`/`blink`、`text-decoration`
/// ショートハンドの`text-decoration-style`/`text-decoration-color`部分は非対応。
fn parse_text_decoration_line<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TextDecorationLine, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return Ok(TextDecorationLine::default());
    }

    let mut line = TextDecorationLine::default();
    loop {
        let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) else {
            break;
        };
        match_ignore_ascii_case! { &ident,
            "underline" => line.underline = true,
            "line-through" => line.line_through = true,
            _ => return Err(input.new_custom_error(())),
        }
    }
    Ok(line)
}

fn parse_length_percentage<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedLengthPercentage, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    match token {
        Token::Percentage { unit_value, .. } => {
            Ok(SpecifiedLengthPercentage::Percentage(unit_value))
        }
        Token::Number { value: 0.0, .. } => {
            Ok(SpecifiedLengthPercentage::Length(SpecifiedLength::Px(0.0)))
        }
        Token::Dimension {
            value, ref unit, ..
        } => Ok(SpecifiedLengthPercentage::Length(parse_length_unit(
            input, value, unit,
        )?)),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_length_percentage_or_auto<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedLengthPercentageOrAuto, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(SpecifiedLengthPercentageOrAuto::Auto);
    }
    parse_length_percentage(input).map(SpecifiedLengthPercentageOrAuto::LengthPercentage)
}

fn parse_length<'i>(input: &mut Parser<'i, '_>) -> Result<SpecifiedLength, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    match token {
        Token::Number { value: 0.0, .. } => Ok(SpecifiedLength::Px(0.0)),
        Token::Dimension {
            value, ref unit, ..
        } => parse_length_unit(input, value, unit),
        _ => Err(input.new_custom_error(())),
    }
}

/// `<数値><単位>`の単位部分を見て`px`/`em`/`rem`のいずれかとして解釈する。
/// それ以外の単位(`pt`/`vh`等)はM1では非対応。
fn parse_length_unit<'i>(
    input: &Parser<'i, '_>,
    value: f32,
    unit: &str,
) -> Result<SpecifiedLength, ParseError<'i, ()>> {
    if unit.eq_ignore_ascii_case("px") {
        Ok(SpecifiedLength::Px(value))
    } else if unit.eq_ignore_ascii_case("em") {
        Ok(SpecifiedLength::Em(value))
    } else if unit.eq_ignore_ascii_case("rem") {
        Ok(SpecifiedLength::Rem(value))
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `lab()`/`lch()`/`oklab()`/`oklch()`はCIE系色空間からsRGBへの変換
/// (行列演算・ガンマ補正)の実装が必要で、請求書・帳票用途での実用性に対して
/// 実装コストが見合わないため非対応(`hsl()`/`hwb()`はcssparser-colorが
/// sRGB変換関数を公開しているため対応する)。
fn parse_color<'i>(input: &mut Parser<'i, '_>) -> Result<Color, ParseError<'i, ()>> {
    let color = cssparser_color::Color::parse(input).map_err(|_| input.new_custom_error(()))?;
    match color {
        cssparser_color::Color::CurrentColor => Ok(Color::CurrentColor),
        cssparser_color::Color::Rgba(rgba) => Ok(Color::Rgba {
            red: rgba.red,
            green: rgba.green,
            blue: rgba.blue,
            alpha: rgba.alpha,
        }),
        cssparser_color::Color::Hsl(hsl) => {
            let (r, g, b) = cssparser_color::hsl_to_rgb(
                hsl.hue.unwrap_or(0.0) / 360.0,
                hsl.saturation.unwrap_or(0.0),
                hsl.lightness.unwrap_or(0.0),
            );
            Ok(rgba_from_unit_floats(r, g, b, hsl.alpha.unwrap_or(1.0)))
        }
        cssparser_color::Color::Hwb(hwb) => {
            let (r, g, b) = cssparser_color::hwb_to_rgb(
                hwb.hue.unwrap_or(0.0) / 360.0,
                hwb.whiteness.unwrap_or(0.0),
                hwb.blackness.unwrap_or(0.0),
            );
            Ok(rgba_from_unit_floats(r, g, b, hwb.alpha.unwrap_or(1.0)))
        }
        _ => Err(input.new_custom_error(())),
    }
}

/// 0.0〜1.0のRGB成分・アルファ値から[`Color::Rgba`]を組み立てる。
fn rgba_from_unit_floats(red: f32, green: f32, blue: f32, alpha: f32) -> Color {
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::Rgba {
        red: to_u8(red),
        green: to_u8(green),
        blue: to_u8(blue),
        alpha: alpha.clamp(0.0, 1.0),
    }
}

/// `content`。文字列リテラル・`attr()`・`counter()`/`counters()`・引用符
/// キーワードの列を受け付け、任意個連結できる([0024](
/// ../../../docs/decisions/0024-generated-content-design.md)決定1)。
/// `none`/`normal`は「生成ボックスなし」を表す`None`として扱う。
fn parse_content<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Option<Vec<ContentPart>>, ParseError<'i, ()>> {
    if input
        .try_parse(|input| -> Result<(), ParseError<'i, ()>> {
            let ident = input.expect_ident()?.clone();
            match_ignore_ascii_case! { &ident,
                "none" | "normal" => Ok(()),
                _ => Err(input.new_custom_error(())),
            }
        })
        .is_ok()
    {
        return Ok(None);
    }

    let mut parts = Vec::new();
    loop {
        if let Ok(s) = input.try_parse(|input| input.expect_string_cloned()) {
            parts.push(ContentPart::String(s.as_ref().to_string()));
            continue;
        }
        if let Ok(part) = input.try_parse(parse_content_quote_keyword) {
            parts.push(part);
            continue;
        }
        if let Ok(part) = input.try_parse(parse_content_function) {
            parts.push(part);
            continue;
        }
        break;
    }
    if parts.is_empty() {
        return Err(input.new_custom_error(()));
    }
    Ok(Some(parts))
}

fn parse_content_quote_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ContentPart, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "open-quote" => ContentPart::OpenQuote,
        "close-quote" => ContentPart::CloseQuote,
        "no-open-quote" => ContentPart::NoOpenQuote,
        "no-close-quote" => ContentPart::NoCloseQuote,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `attr(name)`/`counter(name [, style])`/`counters(name, separator [, style])`。
fn parse_content_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ContentPart, ParseError<'i, ()>> {
    let name = input.expect_function()?.clone();
    if name.eq_ignore_ascii_case("attr") {
        return input.parse_nested_block(|input| {
            let ident = input.expect_ident()?.clone();
            Ok(ContentPart::Attr(ident.as_ref().to_string()))
        });
    }
    if name.eq_ignore_ascii_case("counter") {
        return input.parse_nested_block(|input| {
            let counter_name = input.expect_ident()?.as_ref().to_string();
            let style = if input.try_parse(|input| input.expect_comma()).is_ok() {
                parse_list_style_type(input)?
            } else {
                ListStyleType::Decimal
            };
            Ok(ContentPart::Counter(counter_name, style))
        });
    }
    if name.eq_ignore_ascii_case("counters") {
        return input.parse_nested_block(|input| {
            let counter_name = input.expect_ident()?.as_ref().to_string();
            input.expect_comma()?;
            let separator = input.expect_string()?.as_ref().to_string();
            let style = if input.try_parse(|input| input.expect_comma()).is_ok() {
                parse_list_style_type(input)?
            } else {
                ListStyleType::Decimal
            };
            Ok(ContentPart::Counters(counter_name, separator, style))
        });
    }
    Err(input.new_custom_error(()))
}

/// `counter-reset`/`counter-increment`共通。`none`は空リスト、それ以外は
/// `name [<integer>]`の繰り返し(値省略時は`default_value`)。
fn parse_counter_list<'i>(
    input: &mut Parser<'i, '_>,
    default_value: i32,
) -> Result<Vec<(String, i32)>, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    loop {
        let Ok(name) = input.try_parse(|input| input.expect_ident_cloned()) else {
            break;
        };
        let value = input
            .try_parse(|input| input.expect_integer())
            .unwrap_or(default_value);
        result.push((name.as_ref().to_string(), value));
    }
    if result.is_empty() {
        return Err(input.new_custom_error(()));
    }
    Ok(result)
}

/// `quotes`。`none`は`None`(常に空文字列を生成する、[0024]決定3)、それ以外は
/// `"開き" "閉じ"`のペアの繰り返し(ネスト深度が浅い順)。
fn parse_quotes<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Option<Vec<QuotePair>>, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return Ok(None);
    }
    let mut pairs = Vec::new();
    loop {
        let Ok(open) = input.try_parse(|input| input.expect_string_cloned()) else {
            break;
        };
        let close = input.expect_string()?.as_ref().to_string();
        pairs.push(QuotePair {
            open: open.as_ref().to_string(),
            close,
        });
    }
    if pairs.is_empty() {
        return Err(input.new_custom_error(()));
    }
    Ok(Some(pairs))
}

/// `background-image`の簡易実装。`url(...)`1つのみ受け付ける
/// (`linear-gradient()`等の非`url()`値、複数背景のカンマ区切りは非対応)。
/// `none`は「背景画像なし」を表す`None`として扱う。
fn parse_background_image<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Option<String>, ParseError<'i, ()>> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return match_ignore_ascii_case! { &ident,
            "none" => Ok(None),
            _ => Err(input.new_custom_error(())),
        };
    }
    let url = input
        .expect_url_or_string()
        .map_err(|_| input.new_custom_error(()))?;
    Ok(Some(url.as_ref().to_string()))
}

/// `background-position`の1コンポーネント。`left`/`right`は水平軸、
/// `top`/`bottom`は垂直軸を確定させる。`center`・長さ・パーセンテージは
/// どちらの軸にもなり得る([0025](../../../docs/decisions/0025-background-details-design.md)
/// 決定2)。
enum BackgroundPositionComponent {
    Horizontal(SpecifiedLengthPercentage),
    Vertical(SpecifiedLengthPercentage),
    Either(SpecifiedLengthPercentage),
}

fn parse_background_position_component<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundPositionComponent, ParseError<'i, ()>> {
    use BackgroundPositionComponent as C;
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return match_ignore_ascii_case! { &ident,
            "left" => Ok(C::Horizontal(SpecifiedLengthPercentage::Percentage(0.0))),
            "right" => Ok(C::Horizontal(SpecifiedLengthPercentage::Percentage(1.0))),
            "top" => Ok(C::Vertical(SpecifiedLengthPercentage::Percentage(0.0))),
            "bottom" => Ok(C::Vertical(SpecifiedLengthPercentage::Percentage(1.0))),
            "center" => Ok(C::Either(SpecifiedLengthPercentage::Percentage(0.5))),
            _ => Err(input.new_custom_error(())),
        };
    }
    Ok(C::Either(parse_length_percentage(input)?))
}

/// `background-position`。1〜2値、キーワード(`left`/`center`/`right`/`top`/
/// `bottom`)と長さ・パーセンテージの組み合わせを受け付ける(決定2)。
fn parse_background_position<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedBackgroundPosition, ParseError<'i, ()>> {
    use BackgroundPositionComponent as C;

    let half = SpecifiedLengthPercentage::Percentage(0.5);
    let first = parse_background_position_component(input)?;
    let second = input.try_parse(parse_background_position_component).ok();

    let (horizontal, vertical) = match second {
        None => match first {
            C::Vertical(v) => (half, v),
            C::Horizontal(h) | C::Either(h) => (h, half),
        },
        Some(second) => {
            let first_is_vertical = matches!(first, C::Vertical(_));
            let second_is_horizontal = matches!(second, C::Horizontal(_));
            if matches!((&first, &second), (C::Horizontal(_), C::Horizontal(_)))
                || matches!((&first, &second), (C::Vertical(_), C::Vertical(_)))
            {
                return Err(input.new_custom_error(()));
            }
            let value_of = |c: C| match c {
                C::Horizontal(v) | C::Vertical(v) | C::Either(v) => v,
            };
            if first_is_vertical || second_is_horizontal {
                (value_of(second), value_of(first))
            } else {
                (value_of(first), value_of(second))
            }
        }
    };

    Ok(SpecifiedBackgroundPosition {
        horizontal,
        vertical,
    })
}

/// `background-size`。`cover`/`contain`、または`[<length-percentage> |
/// auto]{1,2}`(1値のみの場合、高さは`auto`)。
fn parse_background_size<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SpecifiedBackgroundSize, ParseError<'i, ()>> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return match_ignore_ascii_case! { &ident,
            "cover" => Ok(SpecifiedBackgroundSize::Cover),
            "contain" => Ok(SpecifiedBackgroundSize::Contain),
            "auto" => Ok(SpecifiedBackgroundSize::WidthHeight(
                SpecifiedLengthPercentageOrAuto::Auto,
                input
                    .try_parse(parse_length_percentage_or_auto)
                    .unwrap_or(SpecifiedLengthPercentageOrAuto::Auto),
            )),
            _ => Err(input.new_custom_error(())),
        };
    }
    let width = SpecifiedLengthPercentageOrAuto::LengthPercentage(parse_length_percentage(input)?);
    let height = input
        .try_parse(parse_length_percentage_or_auto)
        .unwrap_or(SpecifiedLengthPercentageOrAuto::Auto);
    Ok(SpecifiedBackgroundSize::WidthHeight(width, height))
}

/// `background-repeat`。CSS2.1の値集合のみ(`round`/`space`等CSS3値・
/// カンマ区切りの複数背景は非対応)。
fn parse_background_repeat<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundRepeat, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "repeat" => BackgroundRepeat::Repeat,
        "repeat-x" => BackgroundRepeat::RepeatX,
        "repeat-y" => BackgroundRepeat::RepeatY,
        "no-repeat" => BackgroundRepeat::NoRepeat,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `background-attachment`。`fixed`は`scroll`と同一視して描画する(決定5)。
fn parse_background_attachment<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundAttachment, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "scroll" => BackgroundAttachment::Scroll,
        "fixed" => BackgroundAttachment::Fixed,
        _ => return Err(input.new_custom_error(())),
    })
}

/// `background`ショートハンドの簡易実装。`color`/`image`/`repeat`/
/// `attachment`/`position`(`/`区切りで直後に`size`)を任意の順序で受け付ける
/// (`border`ショートハンドと同じ「ループでどの種類の値か`try_parse`で判定」
/// 方式)。仕様通り、指定されなかったロングハンドは全て初期値へリセットする
/// (決定6。`border`/`list-style`ショートハンドとは異なり、以前の宣言を
/// 引きずらない)。
fn parse_background_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let mut color = None;
    let mut image = None;
    let mut repeat = None;
    let mut attachment = None;
    let mut position = None;
    let mut size = None;

    loop {
        if position.is_none() {
            if let Ok(p) = input.try_parse(parse_background_position) {
                position = Some(p);
                if input.try_parse(|input| input.expect_delim('/')).is_ok() {
                    size = Some(parse_background_size(input)?);
                }
                continue;
            }
        }
        if repeat.is_none() {
            if let Ok(r) = input.try_parse(parse_background_repeat) {
                repeat = Some(r);
                continue;
            }
        }
        if attachment.is_none() {
            if let Ok(a) = input.try_parse(parse_background_attachment) {
                attachment = Some(a);
                continue;
            }
        }
        if image.is_none() {
            if let Ok(img) = input.try_parse(parse_background_image) {
                image = Some(img);
                continue;
            }
        }
        if color.is_none() {
            if let Ok(c) = input.try_parse(parse_color) {
                color = Some(c);
                continue;
            }
        }
        break;
    }

    Ok(vec![
        D::BackgroundColor(color.unwrap_or(Color::Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0.0,
        })),
        D::BackgroundImage(image.unwrap_or(None)),
        D::BackgroundPosition(position.unwrap_or(SpecifiedBackgroundPosition {
            horizontal: SpecifiedLengthPercentage::Percentage(0.0),
            vertical: SpecifiedLengthPercentage::Percentage(0.0),
        })),
        D::BackgroundSize(size.unwrap_or(SpecifiedBackgroundSize::WidthHeight(
            SpecifiedLengthPercentageOrAuto::Auto,
            SpecifiedLengthPercentageOrAuto::Auto,
        ))),
        D::BackgroundRepeat(repeat.unwrap_or(BackgroundRepeat::Repeat)),
        D::BackgroundAttachment(attachment.unwrap_or(BackgroundAttachment::Scroll)),
    ])
}

fn parse_font_family<'i>(input: &mut Parser<'i, '_>) -> Result<Vec<String>, ParseError<'i, ()>> {
    input.parse_comma_separated(parse_family_name)
}

/// 単一の`<family-name>`(引用符付き文字列、または空白区切りの識別子の連なり)を
/// パースする。`font-family`プロパティ(カンマ区切りリスト)と`@font-face`の
/// `font-family`ディスクリプタ(単一値)の両方から呼ばれる。
pub(crate) fn parse_family_name<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<String, ParseError<'i, ()>> {
    if let Ok(name) = input.try_parse(|input| input.expect_string_cloned()) {
        return Ok(name.as_ref().to_string());
    }
    let mut name = input.expect_ident()?.as_ref().to_string();
    while let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        name.push(' ');
        name.push_str(&ident);
    }
    Ok(name)
}
