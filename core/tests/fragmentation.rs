//! 改ページパターンごとのゴールデンPDF比較テスト。
//!
//! `paginate.rs`のユニットテストが改ページアルゴリズム自体を
//! 網羅的に検証しているのに対し、こちらは実際のパイプライン全体
//! (HTMLパース → スタイルカスケード → ページ分割 → PDFエンコード)を通して、
//! 各改ページパターンが最終的なPDF出力まで正しく反映されること(および
//! 将来の回帰)を検知するのが目的。
//!
//! 比較粒度は、PDFバイト列の完全一致(埋め込みフォントやオブジェクト番号の
//! 割り当てでずれやすく壊れやすい)ではなく、`/MediaBox`の出現数から数えた
//! ページ数を採用する(`paginate_document`が返すページ数と、実際に書き出した
//! PDFのページ数が一致することも合わせて確認する)。`break-inside: avoid`の
//! ように「ページ数自体は変わらないが配置が変わる」パターンの詳細な検証は
//! `paginate.rs`のユニットテストに譲り、ここではパイプライン全体が
//! クラッシュせず妥当なページ数のPDFを生成できることの確認に留める。

use std::collections::HashMap;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, paginate_document, LaidOutBox, LaidOutContent, PageSettings,
};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn test_fonts() -> FontCollection {
    FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ])
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn page_count_in_pdf(bytes: &[u8]) -> usize {
    count_occurrences(bytes, b"/MediaBox")
}

/// HTML+CSSから、実際のパイプライン(パース→カスケード→ページ分割→PDF
/// エンコード)を一通り実行する。`paginate_document`が返すページ数と、
/// 実際に書き出したPDFバイト列から数えたページ数の両方を返す。
fn build_pdf(html_src: &str, css: &str) -> (usize, Vec<u8>) {
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(css);
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let settings = PageSettings::default();

    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let engine_page_count = pages.len();
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(count_occurrences(&bytes, b"%%EOF") > 0);
    assert_eq!(
        page_count_in_pdf(&bytes),
        engine_page_count,
        "PDF page count should match the layout engine's own page count"
    );

    (engine_page_count, bytes)
}

#[test]
fn break_before_always_forces_a_second_page_end_to_end() {
    let html_src = r#"<div><p class="a">A</p><p class="b">B</p></div>"#;

    let (without, _) = build_pdf(html_src, ".a, .b { height: 50px; margin: 0; }");
    assert_eq!(
        without, 1,
        "without break-before, both tiny paragraphs should fit on one page"
    );

    let (with, _) = build_pdf(
        html_src,
        ".a, .b { height: 50px; margin: 0; } \
         .b { break-before: always; }",
    );
    assert_eq!(
        with, 2,
        "break-before: always should force a second page end-to-end"
    );
}

#[test]
fn break_after_always_forces_a_second_page_end_to_end() {
    let html_src = r#"<div><p class="a">A</p><p class="b">B</p></div>"#;

    let (without, _) = build_pdf(html_src, ".a, .b { height: 50px; margin: 0; }");
    assert_eq!(without, 1);

    let (with, _) = build_pdf(
        html_src,
        ".a, .b { height: 50px; margin: 0; } \
         .a { break-after: always; }",
    );
    assert_eq!(
        with, 2,
        "break-after: always should force a second page end-to-end"
    );
}

#[test]
fn break_inside_avoid_renders_a_valid_multi_page_pdf_end_to_end() {
    // break-inside: avoidは「どのページに何が置かれるか」を変えるだけで
    // 総ページ数自体は変わらない場合が多い(詳細な検証は`paginate.rs`の
    // ユニットテスト参照)。ここではパイプライン全体がこのCSSの組み合わせで
    // 破綻しないこと・想定通りのページ数になることを確認する。
    let settings = PageSettings::default();
    let filler_height = settings.content_height() - 200.0;
    let html_src = r#"<div class="filler"></div>
           <div class="wrapper">
               <p class="a">A</p><p class="b">B</p><p class="c">C</p><p class="d">D</p>
           </div>"#;
    let css = format!(
        ".filler {{ height: {filler_height}px; margin: 0; }} \
         .wrapper {{ break-inside: avoid; margin: 0; }} \
         .a, .b, .c, .d {{ height: 100px; margin: 0; }}"
    );

    let (page_count, bytes) = build_pdf(html_src, &css);
    assert_eq!(page_count, 2);
    assert!(
        count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0,
        "font should still be embedded"
    );
}

/// `word_count`語からなる段落が、明示`width`(px)でどう行分割されるかを
/// 測定する(行数と、各行の高さが一様であること)。`paginate.rs`のユニット
/// テストにある同名ヘルパーと同じ考え方: この一様な行高さを基準に`filler`の
/// 高さを逆算し、ページ内の自然な分割点を狙い撃つ。
fn measure_paragraph_lines(word_count: usize, width: f32) -> (usize, f32) {
    let words: Vec<String> = (0..word_count).map(|i| format!("word{i}")).collect();
    let html_src = format!(r#"<p class="target">{}</p>"#, words.join(" "));
    let dom = html::parse(html_src.as_bytes());
    let ua = user_agent_stylesheet();
    let author = parse_stylesheet(&format!(".target {{ width: {width}px; margin: 0; }}"));
    let styles = compute_styles(&dom, &ua, &author);
    let fonts = test_fonts();
    let tree = build_box_tree(&dom, &styles);
    let laid = layout_document(
        &tree,
        &styles,
        &fonts,
        PageSettings::default().content_width(),
    );

    let p = find_tag(&dom, dom.document(), "p").expect("p not found");
    let p_box = find_laid_out(&laid, p).expect("p box not found");
    let LaidOutContent::Inline(lines) = &p_box.content else {
        panic!("expected inline content");
    };
    let height = lines[0].rect.height;
    assert!(
        lines.iter().all(|l| (l.rect.height - height).abs() < 0.01),
        "this test relies on every wrapped line having the same height"
    );
    (lines.len(), height)
}

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
        for child in children {
            if let Some(found) = find_laid_out(child, target) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn orphans_forces_a_second_page_end_to_end() {
    // `paginate.rs`のユニットテスト
    // (`orphans_defers_the_whole_paragraph_when_too_few_lines_would_fit`)と
    // 同じシナリオ(自然には1行しか収まらず、orphans: 3を満たせない)を
    // パイプライン全体(PDFエンコードまで)を通して再確認する。
    //
    // 総ページ数だけでは「orphansが実際に効いたか」までは証明できない
    // (行が全く収まらない場合、orphansの有無に関わらずどのみち2ページに
    // 分かれるため)。ページ内の配置がどう変わったかの詳細な検証は
    // `paginate.rs`のユニットテスト側に譲り、ここでは「この組み合わせで
    // パイプライン全体が破綻せず、想定通りのページ数になる」ことの
    // 回帰検知に留める。
    let word_count = 60;
    let width = 200.0;
    let (n, line_height) = measure_paragraph_lines(word_count, width);
    assert!(n >= 4, "expected several wrapped lines, got {n}");

    let settings = PageSettings::default();
    let target_fit = 1usize;
    let orphans = 3;
    let desired_remaining = (target_fit as f32 + 0.5) * line_height;
    let filler_height = settings.content_height() - 8.0 - desired_remaining;

    let words: Vec<String> = (0..word_count).map(|i| format!("word{i}")).collect();
    let html_src = format!(
        r#"<div class="filler"></div><p class="target">{}</p>"#,
        words.join(" ")
    );
    let css = format!(
        ".filler {{ height: {filler_height}px; margin: 0; }} \
         .target {{ width: {width}px; margin: 0; orphans: {orphans}; }}"
    );

    let (page_count, _) = build_pdf(&html_src, &css);
    assert_eq!(
        page_count, 2,
        "orphans: {orphans} should force the whole paragraph onto a second page end-to-end"
    );
}

#[test]
fn widows_forces_lines_forward_end_to_end() {
    let word_count = 60;
    let width = 200.0;
    let (n, line_height) = measure_paragraph_lines(word_count, width);
    assert!(n >= 8, "expected several wrapped lines, got {n}");

    let settings = PageSettings::default();
    // 自然には(n - 1)行がこのページに収まり、次ページには1行しか残らない想定
    // (widows: 3を満たせないため、分割点が繰り上がるはず)。orphansのテスト
    // 同様、ここでは詳細な分割点の検証ではなく、この組み合わせでパイプライン
    // 全体が破綻せず想定通りのページ数になることの回帰検知に留める。
    let target_fit = n - 1;
    let desired_remaining = (target_fit as f32 + 0.5) * line_height;
    let filler_height = settings.content_height() - 8.0 - desired_remaining;

    let words: Vec<String> = (0..word_count).map(|i| format!("word{i}")).collect();
    let html_src = format!(
        r#"<div class="filler"></div><p class="target">{}</p>"#,
        words.join(" ")
    );
    let css = format!(
        ".filler {{ height: {filler_height}px; margin: 0; }} \
         .target {{ width: {width}px; margin: 0; widows: 3; }}"
    );

    let (page_count, _) = build_pdf(&html_src, &css);
    assert_eq!(
        page_count, 2,
        "the paragraph should still split across exactly two pages"
    );
}
