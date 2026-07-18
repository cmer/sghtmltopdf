//! Inline Formatting Context: 単純な貪欲法によるテキストの行分割と行ボックスの配置。
//!
//! 既知の簡略化(将来のマイルストーンで見直す):
//! - `white-space: normal`相当の折り返し(連続する空白の畳み込み、単語単位の折り返し)
//!   のみ対応。長い単語1つで行幅を超える場合でも単語内では分割しない
//! - インライン要素(`<b>`等)の境界はT5のボックスツリー構築時点でテキストに
//!   平坦化済みのため、行内での要素ごとの装飾(太字など)はM1では区別されない
//! - 文書全体で単一のフォントを使う(`font-family`によるフォント切り替えは非対応)

use crate::fonts::{measure_text, Font};
use crate::style::ComputedStyle;

use super::geometry::Rect;

#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    pub text: String,
    pub rect: Rect,
}

/// `text`を`available_width`に収まるよう行分割し、`(origin_x, origin_y)`を起点に
/// 縦に積んだ行ボックス列を返す。グリフ幅は`font`によるシェイピング結果を使う。
pub fn layout_inline_content(
    text: &str,
    style: &ComputedStyle,
    font: &Font,
    available_width: f32,
    origin_x: f32,
    origin_y: f32,
) -> Vec<LineBox> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let font_size = style.font_size.0;
    let line_height = font_size * 1.2;
    let space_width = measure_text(font, " ", font_size);

    let mut lines = Vec::new();
    let mut current_words: Vec<&str> = Vec::new();
    let mut current_width = 0.0f32;

    for word in words {
        let word_width = measure_text(font, word, font_size);

        if !current_words.is_empty() {
            let width_with_word = current_width + space_width + word_width;
            if width_with_word > available_width {
                lines.push(finish_line(
                    &current_words,
                    current_width,
                    origin_x,
                    origin_y + lines.len() as f32 * line_height,
                    line_height,
                ));
                current_words.clear();
                current_width = 0.0;
            }
        }

        if current_words.is_empty() {
            current_width = word_width;
        } else {
            current_width += space_width + word_width;
        }
        current_words.push(word);
    }

    if !current_words.is_empty() {
        lines.push(finish_line(
            &current_words,
            current_width,
            origin_x,
            origin_y + lines.len() as f32 * line_height,
            line_height,
        ));
    }

    lines
}

fn finish_line(words: &[&str], width: f32, x: f32, y: f32, height: f32) -> LineBox {
    LineBox {
        text: words.join(" "),
        rect: Rect {
            x,
            y,
            width,
            height,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_font() -> Font {
        Font::load(TEST_FONT_PATH).expect("should load bundled test font")
    }

    #[test]
    fn empty_or_whitespace_only_text_produces_no_lines() {
        let style = ComputedStyle::default();
        let font = test_font();
        assert!(layout_inline_content("", &style, &font, 200.0, 0.0, 0.0).is_empty());
        assert!(layout_inline_content("   \n\t  ", &style, &font, 200.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn text_that_fits_stays_on_a_single_line() {
        let style = ComputedStyle::default();
        let font = test_font();
        let lines = layout_inline_content("hello world", &style, &font, 500.0, 10.0, 20.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello world");
        assert_eq!(lines[0].rect.x, 10.0);
        assert_eq!(lines[0].rect.y, 20.0);
        assert!(lines[0].rect.width > 0.0);
        assert_eq!(lines[0].rect.height, style.font_size.0 * 1.2);
    }

    #[test]
    fn wraps_to_a_new_line_when_available_width_is_too_narrow() {
        let style = ComputedStyle::default();
        let font = test_font();

        let one_line =
            layout_inline_content("hello world foo bar", &style, &font, 1000.0, 0.0, 0.0);
        assert_eq!(one_line.len(), 1);

        let wrapped = layout_inline_content("hello world foo bar", &style, &font, 60.0, 0.0, 0.0);
        assert!(wrapped.len() > 1);

        // 2行目のyは1行目のyより行送り分だけ下にあるはず。
        let line_height = style.font_size.0 * 1.2;
        assert_eq!(wrapped[1].rect.y, wrapped[0].rect.y + line_height);
    }

    #[test]
    fn overlong_single_word_is_not_split_and_still_placed() {
        let style = ComputedStyle::default();
        let font = test_font();
        let lines = layout_inline_content(
            "supercalifragilisticexpialidocious",
            &style,
            &font,
            10.0,
            0.0,
            0.0,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "supercalifragilisticexpialidocious");
        assert!(
            lines[0].rect.width > 10.0,
            "overflowing word should not be dropped or split"
        );
    }

    #[test]
    fn collapses_runs_of_whitespace_between_words() {
        let style = ComputedStyle::default();
        let font = test_font();
        let lines = layout_inline_content("a    b\n\tc", &style, &font, 500.0, 0.0, 0.0);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "a b c");
    }
}
