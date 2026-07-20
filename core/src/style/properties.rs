//! `Declaration: value;`のパースと、プロパティ宣言の型。

use cssparser::{match_ignore_ascii_case, CowRcStr, ParseError, Parser, Token};

use super::values::{
    BorderCollapse, BorderStyle, BreakBetween, BreakInside, CaptionSide, Clear, Color, Display,
    EmptyCells, Float, FontStyle, FontWeight, Position, SpecifiedLength, SpecifiedLengthPercentage,
    SpecifiedLengthPercentageOrAuto, SpecifiedLineHeight, SpecifiedSpacing, TableLayout, TextAlign,
    TextDecorationLine, TextTransform, VerticalAlign, WhiteSpace,
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
    BorderTopLeftRadius(SpecifiedLength),
    BorderTopRightRadius(SpecifiedLength),
    BorderBottomRightRadius(SpecifiedLength),
    BorderBottomLeftRadius(SpecifiedLength),
    FontSize(SpecifiedLength),
    FontFamily(Vec<String>),
    FontWeight(FontWeight),
    FontStyle(FontStyle),
    Color(Color),
    BackgroundColor(Color),
    /// `url(...)`(生の値、解決は呼び出し側任せ、`FontFaceSource::Url`と
    /// 同じ方針)。`None`は`none`(背景画像なし)を表す。
    /// `background-position`/`-size`/`-repeat`/`-attachment`は非対応
    /// ([0017](../../../docs/decisions/0017-background-image-design.md)決定3)。
    BackgroundImage(Option<String>),
    TextDecorationLine(TextDecorationLine),
    /// `::before`/`::after`用の`content`。`None`は`none`/`normal`(生成ボックスなし)。
    /// 文字列リテラル1つのみ対応し、`attr()`/`counter()`/複数値の連結は非対応。
    Content(Option<String>),
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
        "padding" => parse_padding_shorthand(input),
        "border" => parse_border_shorthand(input),
        "border-width" => parse_border_width_shorthand(input),
        "border-color" => parse_border_color_shorthand(input),
        "border-style" => parse_border_style_shorthand(input),
        "border-radius" => parse_border_radius_shorthand(input),
        "font-size" => Ok(vec![D::FontSize(parse_length(input)?)]),
        "font-family" => Ok(vec![D::FontFamily(parse_font_family(input)?)]),
        "font-weight" => Ok(vec![D::FontWeight(parse_font_weight(input)?)]),
        "font-style" => Ok(vec![D::FontStyle(parse_font_style(input)?)]),
        "color" => Ok(vec![D::Color(parse_color(input)?)]),
        "background-color" => Ok(vec![D::BackgroundColor(parse_color(input)?)]),
        "background-image" => Ok(vec![D::BackgroundImage(parse_background_image(input)?)]),
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
fn parse_border_radius_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let (top_left, top_right, bottom_right, bottom_left) = parse_four_sides(input, parse_length)?;
    Ok(vec![
        D::BorderTopLeftRadius(top_left),
        D::BorderTopRightRadius(top_right),
        D::BorderBottomRightRadius(bottom_right),
        D::BorderBottomLeftRadius(bottom_left),
    ])
}

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
        _ => return Err(input.new_custom_error(())),
    })
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
        "none" => Display::None,
        _ => return Err(input.new_custom_error(())),
    })
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

/// `content`の簡易実装。文字列リテラル1つのみ受け付ける
/// (`attr()`/`counter()`/複数値の連結/引用符生成は非対応)。
/// `none`/`normal`は「生成ボックスなし」を表す`None`として扱う。
fn parse_content<'i>(input: &mut Parser<'i, '_>) -> Result<Option<String>, ParseError<'i, ()>> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return match_ignore_ascii_case! { &ident,
            "none" | "normal" => Ok(None),
            _ => Err(input.new_custom_error(())),
        };
    }
    Ok(Some(input.expect_string()?.as_ref().to_string()))
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
