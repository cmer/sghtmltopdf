//! `Declaration: value;`のパースと、プロパティ宣言の型。

use cssparser::{match_ignore_ascii_case, CowRcStr, ParseError, Parser, Token};

use super::values::{Color, Display, Length, LengthPercentage, LengthPercentageOrAuto};

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyDeclaration {
    Display(Display),
    Width(LengthPercentageOrAuto),
    Height(LengthPercentageOrAuto),
    MarginTop(LengthPercentageOrAuto),
    MarginRight(LengthPercentageOrAuto),
    MarginBottom(LengthPercentageOrAuto),
    MarginLeft(LengthPercentageOrAuto),
    PaddingTop(LengthPercentage),
    PaddingRight(LengthPercentage),
    PaddingBottom(LengthPercentage),
    PaddingLeft(LengthPercentage),
    BorderTopWidth(Length),
    BorderRightWidth(Length),
    BorderBottomWidth(Length),
    BorderLeftWidth(Length),
    FontSize(Length),
    FontFamily(Vec<String>),
    Color(Color),
    BackgroundColor(Color),
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
        "font-size" => Ok(vec![D::FontSize(parse_length(input)?)]),
        "font-family" => Ok(vec![D::FontFamily(parse_font_family(input)?)]),
        "color" => Ok(vec![D::Color(parse_color(input)?)]),
        "background-color" => Ok(vec![D::BackgroundColor(parse_color(input)?)]),
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

/// `border`ショートハンドの簡易実装。
/// M1ではレイアウト計算に必要な`border-width`のみ抽出し、
/// `border-style`/`border-color`(キーワードや色)は読み飛ばす。
fn parse_border_shorthand<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<PropertyDeclaration>, ParseError<'i, ()>> {
    use PropertyDeclaration as D;
    let mut width = Length(0.0);
    let mut found_width = false;

    loop {
        if !found_width {
            if let Ok(w) = input.try_parse(parse_length) {
                width = w;
                found_width = true;
                continue;
            }
        }
        // border-style(solid等)やborder-colorは読み飛ばす。
        if input.try_parse(|input| input.expect_ident_cloned()).is_ok() {
            continue;
        }
        if input.try_parse(parse_color).is_ok() {
            continue;
        }
        break;
    }

    Ok(vec![
        D::BorderTopWidth(width),
        D::BorderRightWidth(width),
        D::BorderBottomWidth(width),
        D::BorderLeftWidth(width),
    ])
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

fn parse_length_percentage<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthPercentage, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    match token {
        Token::Dimension {
            value, ref unit, ..
        } if unit.eq_ignore_ascii_case("px") => Ok(LengthPercentage::Length(value)),
        Token::Percentage { unit_value, .. } => Ok(LengthPercentage::Percentage(unit_value)),
        Token::Number { value: 0.0, .. } => Ok(LengthPercentage::Length(0.0)),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_length_percentage_or_auto<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthPercentageOrAuto, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(LengthPercentageOrAuto::Auto);
    }
    parse_length_percentage(input).map(LengthPercentageOrAuto::LengthPercentage)
}

fn parse_length<'i>(input: &mut Parser<'i, '_>) -> Result<Length, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    match token {
        Token::Dimension {
            value, ref unit, ..
        } if unit.eq_ignore_ascii_case("px") => Ok(Length(value)),
        Token::Number { value: 0.0, .. } => Ok(Length(0.0)),
        _ => Err(input.new_custom_error(())),
    }
}

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
        _ => Err(input.new_custom_error(())),
    }
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
