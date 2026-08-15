//! `&nbsp;`をはじめとするUnicodeの空白文字のE2Eテスト。
//!
//! `typography.rs`と同じ方針で、実際のパイプライン(HTMLパース→スタイル
//! カスケード→レイアウト)を通して検証する。判定の基準は
//! `layout::white_space`と同じ2軸:
//!
//! - 畳み込まれるのはCSS Text 3の対象(space/tab/改行)だけで、それ以外の
//!   空白は普通の文字としてフォント本来の字幅で描かれる。
//! - 改行してよい位置はUAX #14の行分割クラスに従う(`&nbsp;`は不可、
//!   thin space・ZWSPは直後で可)。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, paginate_document, LaidOutBox, LaidOutContent, PageSettings,
};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn find_tag(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
    if let NodeData::Element { name, .. } = &dom.node(id).data {
        if &*name.local == tag {
            return Some(id);
        }
    }
    dom.children(id).find_map(|child| find_tag(dom, child, tag))
}

fn find_laid_out(b: &LaidOutBox, target: NodeId) -> Option<&LaidOutBox> {
    if b.node == Some(target) {
        return Some(b);
    }
    if let LaidOutContent::Blocks(children) = &b.content {
        return children.iter().find_map(|c| find_laid_out(c, target));
    }
    None
}

/// 最初の`<p>`が組んだ行を`(幅, テキスト)`の並びで返す。
fn p_lines(html_src: &str, css: &str) -> Vec<(f32, String)> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let fonts = FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ]);
    let tree = build_box_tree(&dom, &styles);
    let laid = layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    );
    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p should be laid out");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    lines
        .iter()
        .map(|line| {
            (
                line.rect.width,
                line.runs.iter().map(|r| r.text.as_str()).collect(),
            )
        })
        .collect()
}

/// 折り返さない幅で1行に組んだときの幅。
fn width_of(html_src: &str) -> f32 {
    let lines = p_lines(html_src, "body { margin: 0; } p { margin: 0; }");
    assert_eq!(lines.len(), 1, "expected a single line, got {lines:?}");
    lines[0].0
}

/// 40px幅の`<p>`に組んだときの行数(改行機会の有無を見るため)。
fn narrow_lines(html_src: &str) -> Vec<(f32, String)> {
    p_lines(
        html_src,
        "body { margin: 0; } p { margin: 0; width: 40px; }",
    )
}

// ===== 畳み込み =====

#[test]
fn runs_of_ordinary_spaces_still_collapse_into_one() {
    assert_eq!(
        width_of("<p>a   b</p>"),
        width_of("<p>a b</p>"),
        "space/tab/newline are the only collapsible characters"
    );
    assert_eq!(width_of("<p>a \t\n b</p>"), width_of("<p>a b</p>"));
}

#[test]
fn a_run_of_no_break_spaces_does_not_collapse() {
    // `&nbsp;&nbsp;&nbsp;`は空白3個分の幅を占める(桁揃えに使われる)。
    // 畳み込んでいた頃は普通の空白1個と同じ幅になっていた。
    let one_space = width_of("<p>a b</p>");
    let three_nbsp = width_of("<p>a\u{a0}\u{a0}\u{a0}b</p>");

    assert!(
        three_nbsp > one_space + 1.0,
        "three &nbsp; must be wider than a single space ({three_nbsp} vs {one_space})"
    );
}

#[test]
fn a_no_break_space_is_as_wide_as_a_space() {
    // DejaVu Sansでは`&nbsp;`とspaceのアドバンスが等しい。フォントがグリフを
    // 持たない場合もシェイパーのspace fallbackが同じ幅を割り当てる。
    assert_eq!(width_of("<p>a\u{a0}b</p>"), width_of("<p>a b</p>"));
}

#[test]
fn fixed_width_spaces_keep_their_own_advance() {
    // 整形用スペースは「空白1個」に均されず、それぞれの字幅で描かれる。
    let none = width_of("<p>ab</p>");
    let hair = width_of("<p>a\u{200a}b</p>");
    let thin = width_of("<p>a\u{2009}b</p>");
    let space = width_of("<p>a b</p>");
    let em = width_of("<p>a\u{2003}b</p>");

    assert!(
        none < hair && hair < thin && thin < space && space < em,
        "expected none < hair < thin < space < em, got \
         {none} / {hair} / {thin} / {space} / {em}"
    );
}

#[test]
fn a_zero_width_space_takes_no_room() {
    assert_eq!(
        width_of("<p>a\u{200b}b</p>"),
        width_of("<p>ab</p>"),
        "U+200B must not add width"
    );
}

// ===== 改行機会(UAX #14) =====

#[test]
fn a_no_break_space_does_not_offer_a_wrap_opportunity() {
    // 「10 kg」は折り返せるが、「10&nbsp;kg」は1行に留まる(はみ出してでも
    // 分断しない)。`&nbsp;`はまさにこれのために置かれる文字。
    assert_eq!(narrow_lines("<p>10 kg</p>").len(), 2);

    let glued = narrow_lines("<p>10\u{a0}kg</p>");
    assert_eq!(glued.len(), 1, "&nbsp; must not break, got {glued:?}");
    assert_eq!(glued[0].1, "10\u{a0}kg");
}

#[test]
fn the_other_non_breaking_spaces_do_not_wrap_either() {
    for (name, ch) in [
        ("NARROW NO-BREAK SPACE", '\u{202f}'),
        ("FIGURE SPACE", '\u{2007}'),
        ("WORD JOINER", '\u{2060}'),
    ] {
        let lines = narrow_lines(&format!("<p>10{ch}kg</p>"));
        assert_eq!(lines.len(), 1, "{name} must not break, got {lines:?}");
    }
}

#[test]
fn a_thin_space_offers_a_wrap_opportunity() {
    // UAX #14でBAクラスの空白は直後で改行してよい。
    let lines = narrow_lines("<p>10\u{2009}kg</p>");
    assert_eq!(lines.len(), 2, "thin space should break, got {lines:?}");
    assert_eq!(lines[1].1, "kg", "the break belongs after the space");
}

#[test]
fn a_zero_width_space_offers_a_wrap_opportunity_inside_a_word() {
    // ZWSPは幅を持たないまま改行機会だけを足す(`<wbr>`相当の使い方)。
    let broken = narrow_lines("<p>aaaaaa\u{200b}bbbbbb</p>");
    let unbroken = narrow_lines("<p>aaaaaabbbbbb</p>");

    assert_eq!(broken.len(), 2, "U+200B should break, got {broken:?}");
    assert_eq!(
        unbroken.len(),
        1,
        "without U+200B the long word overflows instead, got {unbroken:?}"
    );
}

#[test]
fn a_no_break_space_stays_glued_even_under_word_break_break_all() {
    // `word-break: break-all`はどこでも改行してよいが、`&nbsp;`のために置かれた
    // 結合はそれより優先する(ブラウザも同じ扱い)。
    let lines = p_lines(
        "<p>10\u{a0}kg</p>",
        "body { margin: 0; } p { margin: 0; width: 40px; word-break: break-all; }",
    );

    assert!(
        lines.iter().any(|(_, text)| text.contains("0\u{a0}k")),
        "the characters around the &nbsp; must stay on one line, got {lines:?}"
    );
}

// ===== `<wbr>` =====

#[test]
fn wbr_offers_a_wrap_opportunity_inside_a_long_word() {
    // HTML仕様の"line break opportunity"。長い識別子やURLを狙った位置で
    // 折り返すために使われる。
    let broken = narrow_lines("<p>aaaaaa<wbr>bbbbbb</p>");
    let unbroken = narrow_lines("<p>aaaaaabbbbbb</p>");

    assert_eq!(broken.len(), 2, "<wbr> should break, got {broken:?}");
    assert_eq!(broken[0].1, "aaaaaa");
    assert_eq!(broken[1].1, "bbbbbb");
    assert_eq!(
        unbroken.len(),
        1,
        "without <wbr> the long word overflows instead, got {unbroken:?}"
    );
}

#[test]
fn wbr_adds_no_width_when_the_line_does_not_wrap() {
    assert_eq!(
        width_of("<p>aaa<wbr>bbb</p>"),
        width_of("<p>aaabbb</p>"),
        "<wbr> must not change the width of a line that fits"
    );
}

#[test]
fn wbr_only_offers_a_break_it_does_not_force_one() {
    // `<br>`との違い。収まるうちは1行のまま。
    let lines = p_lines("<p>aaa<wbr>bbb</p>", "body { margin: 0; } p { margin: 0; }");
    assert_eq!(lines.len(), 1, "<wbr> is not a forced break, got {lines:?}");
}

#[test]
fn wbr_survives_word_break_keep_all() {
    // `word-break: keep-all`は「単語内で改行しない」指定だが、`<wbr>`は
    // 明示的に置かれた改行機会なので効き続ける(ZWクラスはUAX #14でも
    // 単語の切れ目とは独立に扱われる)。
    let lines = p_lines(
        "<p>aaaaaa<wbr>bbbbbb</p>",
        "body { margin: 0; } p { margin: 0; width: 40px; word-break: keep-all; }",
    );

    assert_eq!(lines.len(), 2, "<wbr> should still break, got {lines:?}");
}

/// PDFのストリームをすべて展開して連結する(`/ToUnicode`
/// CMapを覗くため。`typography.rs`と同じ手順)。
fn decompressed_streams(html_src: &str) -> Vec<u8> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(
        &dom,
        &user_agent_stylesheet(),
        &parse_stylesheet("body { margin: 0; }"),
    );
    let fonts = FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ]);
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find(&bytes[i..], b"stream\n") {
        let start = i + pos + b"stream\n".len();
        let Some(end_rel) = find(&bytes[start..], b"endstream") else {
            break;
        };
        let end = start + end_rel;
        let mut decoder = flate2::read::ZlibDecoder::new(&bytes[start..end]);
        let mut decompressed = Vec::new();
        if std::io::Read::read_to_end(&mut decoder, &mut decompressed).is_ok() {
            out.extend_from_slice(&decompressed);
        }
        i = end + b"endstream".len();
    }
    out
}

#[test]
fn wbr_leaves_no_character_in_the_laid_out_text() {
    // `<wbr>`は改行機会であって文字ではない。ZWSPを文字として流すと、
    // フォントがZWSPのグリフを持たない場合にspaceのグリフで代替されるため、
    // PDFのテキスト層に幽霊の空白が入る(抽出すると`inline word`になる)。
    for html_src in [
        "<p>inline<wbr>word</p>",
        "<p>a<wbr>b<wbr>c</p>",
        "<p>\u{200b}text</p>",
    ] {
        let lines = p_lines(html_src, "body { margin: 0; } p { margin: 0; }");
        assert!(
            lines.iter().all(|(_, text)| !text.contains('\u{200b}')),
            "no run may carry a U+200B for {html_src}, got {lines:?}"
        );
    }
    assert_eq!(
        p_lines("<p>inline<wbr>word</p>", "body { margin: 0; }")[0].1,
        "inlineword",
        "the text either side of a <wbr> is contiguous"
    );
}

#[test]
fn wbr_leaves_nothing_behind_in_the_pdf_text_layer() {
    // 回帰テスト: `<wbr>`をZWSPの「文字」として流していた頃は、フォントが
    // ZWSPのグリフを持たないためspaceのグリフで代替され、`/ToUnicode`が
    // そのグリフをU+200Bに割り当てていた。結果、文書中のすべての空白が
    // U+200Bとして抽出され、コピー&ペーストとテキスト検索が壊れていた。
    let pdf = decompressed_streams("<p>inline<wbr>word stays</p>");
    let text = String::from_utf8_lossy(&pdf);

    assert!(
        !text.contains("<200B>"),
        "<wbr> must not reach the /ToUnicode CMap"
    );
    assert!(
        text.contains("<0020>"),
        "the space glyph must still map to U+0020, got:\n{text}"
    );
}

// ===== ボックス生成 =====

#[test]
fn a_paragraph_holding_only_a_no_break_space_still_produces_a_line() {
    // 空白のみのテキストならボックスを作らないが、`&nbsp;`は内容なので行になる。
    let blank = p_lines("<p> \n </p>", "body { margin: 0; }");
    let nbsp = p_lines("<p>\u{a0}</p>", "body { margin: 0; }");

    assert!(blank.is_empty(), "collapsible whitespace makes no line");
    assert_eq!(nbsp.len(), 1, "&nbsp; is content and makes a line");
}
