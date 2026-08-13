//! 空白文字の分類。
//!
//! Unicodeには`char::is_whitespace`(White_Spaceプロパティ)が真になる文字が
//! 多数あるが、CSSの行組みではそれらを一律に扱ってはいけない。分類は2軸ある。
//!
//! 1. **畳み込むか** — CSS Text 3 §4.1が畳み込みの対象とするのは
//!    space (U+0020)・tab (U+0009)・segment break (U+000A)だけで、それ以外の
//!    Zs(`&nbsp;`やthin spaceなど)は「畳み込まれない普通の文字」として扱う。
//!    Blinkの`Character::IsCollapsibleSpace`(space/LF/tab/CR)、Geckoの
//!    `nsTextFrameUtils::IsSpaceOrTab`(space/tab、改行は別処理)も同じ範囲。
//!    畳み込まない文字は行組みの単語区切りにはならず、そのままシェイピングへ
//!    渡してフォント本来の字幅で描く(`&nbsp;`3個は空白3個分の幅になる)。
//!
//! 2. **その位置で改行してよいか** — UAX #14の行分割クラスによる。
//!    `&nbsp;`(GL)や図形用スペース(GL)は前後で改行してはならず、
//!    thin space等(BA)とZWSP(ZW)は直後で改行してよい。
//!
//! グリフを持たないフォントでも字幅が壊れないのは、シェイパー(harfrust)が
//! HarfBuzzのspace fallback(`_hb_ot_shape_fallback_spaces`)を実装しており、
//! 該当グリフが無ければspaceのグリフで代替して`em/2`・`em/5`といった規定の
//! アドバンスを設定してくれるため。こちら側で幅を用意する必要はない。
//!
//! 既知の簡略化: U+000B・U+0085・U+2028・U+2029は本来「強制改行」だが、
//! グリフを持たないフォントで.notdefが出るのを避けるため、ここでは畳み込む
//! 空白(=単語区切り)として扱う(従来の挙動のまま)。

/// 畳み込みの対象になる空白かどうか(CSS Text 3の"collapsible white space")。
///
/// `white-space: normal`ではこの文字の並びが1個の単語間スペースになり、行頭・
/// 行末では捨てられる。ここに含まれない空白文字(`&nbsp;`等)は普通の文字。
pub(crate) fn is_collapsible(ch: char) -> bool {
    matches!(
        ch,
        '\u{20}'      // SPACE
        | '\u{9}'     // TAB
        | '\u{a}'     // LF(HTMLのsegment break)
        | '\u{d}'     // CR(パーサでLFへ正規化されるが防御的に)
        | '\u{c}'     // FF(HTMLのASCII whitespace)
        | '\u{b}'     // VT
        | '\u{85}'    // NEL
        | '\u{2028}'  // LINE SEPARATOR
        | '\u{2029}' // PARAGRAPH SEPARATOR
    )
}

/// 文字列が畳み込み対象の空白だけでできているか(=そこにボックスを作らなく
/// てよいか)。`str::trim`と違い、`&nbsp;`だけの文字列は「空白のみ」ではない
/// (内容を持つ)と判定する。
pub(crate) fn is_collapsible_only(text: &str) -> bool {
    text.chars().all(is_collapsible)
}

/// 前後で改行してはならない空白かどうか(UAX #14のGL・WJ)。
///
/// `&nbsp;`はまさにこのために使われる文字なので、`word-break: break-all`より
/// 優先して効かせる(ブラウザも「10&nbsp;kg」を分断しない)。
pub(crate) fn is_non_breaking(ch: char) -> bool {
    matches!(
        ch,
        '\u{a0}'      // NO-BREAK SPACE (GL)
        | '\u{2007}'  // FIGURE SPACE (GL、数字の桁揃え用)
        | '\u{202f}'  // NARROW NO-BREAK SPACE (GL)
        | '\u{2060}'  // WORD JOINER (WJ)
        | '\u{feff}' // ZERO WIDTH NO-BREAK SPACE (WJ)
    )
}

/// 直後で改行してよい空白かどうか(UAX #14のBA・ZW)。
///
/// 幅を持つ整形用スペース(thin space等)と、幅を持たないZWSPの両方を含む。
/// U+3000 IDEOGRAPHIC SPACEは`inline::is_cjk`が既に改行機会として扱うため
/// (かつ`word-break: keep-all`の対象であるべきなため)ここには含めない。
pub(crate) fn allows_break_after(ch: char) -> bool {
    matches!(
        ch,
        '\u{1680}'          // OGHAM SPACE MARK (BA)
        | '\u{2000}'..='\u{2006}' // EN QUAD〜SIX-PER-EM SPACE (BA)
        | '\u{2008}'..='\u{200a}' // PUNCTUATION/THIN/HAIR SPACE (BA)
        | '\u{200b}'        // ZERO WIDTH SPACE (ZW)
        | '\u{205f}' // MEDIUM MATHEMATICAL SPACE (BA)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_css_collapsible_set_collapses() {
        for ch in [' ', '\t', '\n', '\r'] {
            assert!(is_collapsible(ch), "{ch:?} should collapse");
        }
        // `char::is_whitespace`は真だが、CSS上は普通の文字。
        for ch in ['\u{a0}', '\u{2009}', '\u{3000}', '\u{202f}', '\u{2007}'] {
            assert!(ch.is_whitespace(), "{ch:?} is Unicode white space");
            assert!(!is_collapsible(ch), "{ch:?} must not collapse");
        }
    }

    #[test]
    fn a_nbsp_only_string_is_not_whitespace_only() {
        assert!(is_collapsible_only(" \n\t "));
        assert!(is_collapsible_only(""));
        assert!(!is_collapsible_only("\u{a0}"));
        assert!(!is_collapsible_only(" \u{2009} "));
    }

    #[test]
    fn glue_and_break_after_classes_do_not_overlap() {
        for ch in ['\u{a0}', '\u{2007}', '\u{202f}', '\u{2060}', '\u{feff}'] {
            assert!(is_non_breaking(ch));
            assert!(!allows_break_after(ch));
        }
        for ch in ['\u{2002}', '\u{2009}', '\u{200a}', '\u{200b}', '\u{205f}'] {
            assert!(allows_break_after(ch));
            assert!(!is_non_breaking(ch));
        }
    }
}
