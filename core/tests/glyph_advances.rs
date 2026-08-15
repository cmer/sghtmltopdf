//! グリフの送り幅がレイアウトと描画で一致することのテスト。
//!
//! レイアウトはシェイパーが返す`x_advance`で位置を決めるが、PDFのビューアは
//! CIDFontの`/W`(グリフIDごとに1つ)でグリフを送る。この2つが食い違う経路が
//! あり、差はTJ配列の補正値で埋めている(`pdf::document::show_run_glyphs`)。
//!
//! ここでは「補正量の合計」が「レイアウト幅と`/W`由来の幅の差」に一致すること
//! を、実際に生成したPDFのコンテンツストリームから確かめる。

use std::collections::HashMap;
use std::io::Read;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf_core::layout::{
    build_box_tree, layout_document, paginate_document, LaidOutBox, LaidOutContent, PageSettings,
};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const DEJAVU: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
/// `&thinsp;`(U+2009)等のグリフを持たないフォント。シェイパーがspaceのグリフで
/// 代替しつつアドバンスだけ差し替えるため、`/W`との食い違いが起きる。
const NOTO_CJK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fonts/NotoSansCJK-Regular.ttc"
);

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

/// 最初の`<p>`について、レイアウトが使う送り幅の合計と、`/W`だけで送った場合の
/// 合計の差(px)を返す。TJの補正が埋めるべき量そのもの。
fn advance_gap_of_first_p(html_src: &str, css: &str, font_path: &str) -> f32 {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let font = Font::load(font_path).expect("should load test font");
    let fonts = FontCollection::new(vec![Font::load(font_path).expect("should load test font")]);
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

    let units_per_em = font.units_per_em() as f32;
    let mut gap = 0.0;
    for line in lines {
        for run in &line.runs {
            for glyph in &run.glyphs {
                let pdf_advance = font.glyph_hor_advance(glyph.glyph_id).unwrap_or(0) as f32
                    * run.font_size
                    / units_per_em;
                gap += glyph.x_advance - pdf_advance;
            }
        }
    }
    gap
}

fn pdf_bytes(html_src: &str, css: &str, font_path: &str) -> Vec<u8> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(css));
    let fonts = FontCollection::new(vec![Font::load(font_path).expect("should load test font")]);
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings)
}

/// コンテンツストリームはFlateDecodeで圧縮されているので、展開して連結する。
fn decompressed_streams(pdf: &[u8]) -> Vec<u8> {
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = find(&pdf[i..], b"stream\n") {
        let start = i + pos + b"stream\n".len();
        let Some(end_rel) = find(&pdf[start..], b"endstream") else {
            break;
        };
        let end = start + end_rel;
        let mut decoded = Vec::new();
        if flate2::read::ZlibDecoder::new(&pdf[start..end])
            .read_to_end(&mut decoded)
            .is_ok()
        {
            out.extend_from_slice(&decoded);
            out.push(b'\n');
        }
        i = end + b"endstream".len();
    }
    out
}

/// `[...] TJ`の配列に現れる補正値をすべて足す(単位はテキスト空間の1/1000)。
///
/// 文字列`(...)`の中はバイト列(エスケープあり)なので読み飛ばす。`TJ`以外の
/// 演算子のオペランドを拾わないよう、`]`の直後が`TJ`である配列だけを数える。
/// 展開したバイト列にはフォントファイルのストリームも混ざっており、そこに
/// 現れた`[`から数え始めると手前の演算子のオペランドまで拾ってしまうため、
/// 途中で`[`が出てきたらそこを配列の開始として数え直す。
fn sum_tj_adjustments(stream: &[u8]) -> f32 {
    fn take_number(buf: &mut String, out: &mut Vec<f32>) {
        if !buf.is_empty() {
            if let Ok(v) = buf.parse::<f32>() {
                out.push(v);
            }
            buf.clear();
        }
    }

    let mut total = 0.0;
    let mut i = 0;
    while i < stream.len() {
        if stream[i] != b'[' {
            i += 1;
            continue;
        }
        let mut numbers = Vec::new();
        let mut buf = String::new();
        let mut in_string = false;
        let mut escaped = false;
        let mut j = i + 1;
        while j < stream.len() {
            let b = stream[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b')' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            if b == b']' {
                take_number(&mut buf, &mut numbers);
                break;
            }
            if b == b'[' {
                // 手前の`[`は配列の開始ではなかった。ここから数え直す。
                numbers.clear();
                buf.clear();
            } else if b == b'(' {
                take_number(&mut buf, &mut numbers);
                in_string = true;
            } else if b.is_ascii_digit() || b == b'-' || b == b'+' || b == b'.' {
                buf.push(b as char);
            } else {
                take_number(&mut buf, &mut numbers);
            }
            j += 1;
        }
        // `]`の後ろの空白を飛ばして演算子を見る。
        let mut k = j + 1;
        while k < stream.len() && stream[k].is_ascii_whitespace() {
            k += 1;
        }
        if stream[k..].starts_with(b"TJ") {
            total += numbers.iter().sum::<f32>();
        }
        i = j + 1;
    }
    total
}

/// TJの補正量の合計(px)。TJの値は送り量から減算されるので符号を反転する。
fn tj_correction_px(html_src: &str, css: &str, font_path: &str, font_size: f32) -> f32 {
    let pdf = pdf_bytes(html_src, css, font_path);
    let stream = decompressed_streams(&pdf);
    -sum_tj_adjustments(&stream) / 1000.0 * font_size
}

const JUSTIFIED: &str = "<p>aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll</p>";
const JUSTIFY_CSS: &str = "body { margin: 0; } \
                           p { margin: 0; text-align: justify; width: 300px; font-size: 16px; }";

#[test]
fn a_justified_line_is_drawn_as_wide_as_it_was_laid_out() {
    // 単語間の隙間は`merge_adjacent_runs`が「隙間ぶんのアドバンスを持つ空白
    // グリフ」として復元する。`text-align: justify`が広げた隙間はspaceの字幅と
    // 一致しないため、TJの補正が無いと行が伸ばした分だけ右端に届かない。
    let expected = advance_gap_of_first_p(JUSTIFIED, JUSTIFY_CSS, DEJAVU);
    assert!(
        expected > 1.0,
        "the test document should actually be stretched, got {expected}px"
    );

    let corrected = tj_correction_px(JUSTIFIED, JUSTIFY_CSS, DEJAVU, 16.0);
    assert!(
        (corrected - expected).abs() < 0.01,
        "TJ should make up the whole stretch: corrected={corrected}px expected={expected}px"
    );
}

#[test]
fn a_left_aligned_line_needs_no_correction() {
    // 両端揃えでなければ隙間はspaceの字幅そのものなので、補正は出ない
    // (通常の文書でTJ配列が無駄に長くならないことの確認でもある)。
    let css = "body { margin: 0; } p { margin: 0; width: 300px; font-size: 16px; }";
    let expected = advance_gap_of_first_p(JUSTIFIED, css, DEJAVU);
    assert!(
        expected.abs() < 0.01,
        "no stretch is expected here, got {expected}px"
    );

    let corrected = tj_correction_px(JUSTIFIED, css, DEJAVU, 16.0);
    assert!(
        corrected.abs() < 0.01,
        "there should be nothing to correct, got {corrected}px"
    );
}

#[test]
fn a_fixed_width_space_the_font_lacks_is_drawn_at_its_own_advance() {
    // フォントが持たない固定幅スペースは、シェイパーがspaceのグリフで代替しつつ
    // アドバンスだけ規定値(U+2009ならem/5)へ差し替える。`/W`は普通のspaceと共有
    // なので、補正が無いと後続の文字がずれる。
    let html_src = "<p>a\u{2009}b\u{2009}c</p>";
    let css = "body { margin: 0; } p { margin: 0; font-size: 16px; }";

    let expected = advance_gap_of_first_p(html_src, css, NOTO_CJK);
    assert!(
        expected.abs() > 0.1,
        "the font should be missing U+2009 for this test to mean anything, got {expected}px"
    );

    let corrected = tj_correction_px(html_src, css, NOTO_CJK, 16.0);
    assert!(
        (corrected - expected).abs() < 0.01,
        "TJ should absorb the difference: corrected={corrected}px expected={expected}px"
    );
}
