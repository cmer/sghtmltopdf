//! `Declaration: value;`のパースと、プロパティ宣言の型。

use cssparser::{match_ignore_ascii_case, CowRcStr, ParseError, Parser, Token};

use super::values::{
    BorderStyle, Color, Display, FontStyle, FontWeight, SpecifiedLength, SpecifiedLengthPercentage,
    SpecifiedLengthPercentageOrAuto,
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
    /// `::before`/`::after`用の`content`。`None`は`none`/`normal`(生成ボックスなし)。
    /// 文字列リテラル1つのみ対応し、`attr()`/`counter()`/複数値の連結は非対応。
    Content(Option<String>),
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
        "content" => Ok(vec![D::Content(parse_content(input)?)]),
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

fn parse_display<'i>(input: &mut Parser<'i, '_>) -> Result<Display, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "block" => Display::Block,
        "inline" => Display::Inline,
        "none" => Display::None,
        _ => return Err(input.new_custom_error(())),
    })
}

/// キーワード(`normal`/`bold`)と数値(`100`〜`900`)のどちらも受け付ける。
/// 数値は600以上を`Bold`とみなす簡略実装(実際の太字フォントを持たず、
/// 描画時に疑似太字で表現するため、細かい太さの段階は区別しない)。
fn parse_font_weight<'i>(input: &mut Parser<'i, '_>) -> Result<FontWeight, ParseError<'i, ()>> {
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

fn parse_font_style<'i>(input: &mut Parser<'i, '_>) -> Result<FontStyle, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    Ok(match_ignore_ascii_case! { &ident,
        "normal" => FontStyle::Normal,
        // `oblique`は専用の傾斜角を持たないため`italic`と同一視する。
        "italic" | "oblique" => FontStyle::Italic,
        _ => return Err(input.new_custom_error(())),
    })
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

fn parse_font_family<'i>(input: &mut Parser<'i, '_>) -> Result<Vec<String>, ParseError<'i, ()>> {
    input.parse_comma_separated(|input| {
        if let Ok(name) = input.try_parse(|input| input.expect_string_cloned()) {
            return Ok(name.as_ref().to_string());
        }
        let mut name = input.expect_ident()?.as_ref().to_string();
        while let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
            name.push(' ');
            name.push_str(&ident);
        }
        Ok(name)
    })
}
