//! `position: absolute`/`fixed`のE2Eテスト(M11 Phase 2、T270)。
//!
//! 設計は[0049](../../docs/decisions/0049-absolute-fixed-positioning-design.md)。
//! 絶対配置は`Mode::Batch`でのみ有効(決定4)。オーバーレイは全ページ確定後に
//! 足すため、ページ分割結果(`paginate_document`)とレイアウト結果で検証する。

use sghtmltopdf_core::engine::{Engine, EngineOptions, FontSpec, Mode};
use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{paginate_document, LaidOutBox, LaidOutContent, PageSettings};
use sghtmltopdf_core::sink::MemorySink;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn test_fonts() -> FontCollection {
    FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ])
}

fn pages_of(html_src: &str) -> Vec<Vec<String>> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);

    fn texts(b: &LaidOutBox, out: &mut Vec<String>) {
        match &b.content {
            LaidOutContent::Inline(lines) => {
                let t: String = lines
                    .iter()
                    .flat_map(|l| l.runs.iter())
                    .map(|r| r.text.as_str())
                    .collect();
                if !t.trim().is_empty() {
                    out.push(t);
                }
            }
            LaidOutContent::Grid(grid) => {
                for c in grid.rows.iter().flat_map(|row| &row.items) {
                    texts(c, out);
                }
            }
            LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
                for c in children {
                    texts(c, out);
                }
            }
            LaidOutContent::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        texts(cell, out);
                    }
                }
            }
            LaidOutContent::Image(_) => {}
        }
    }
    pages
        .iter()
        .map(|page| {
            let mut out = Vec::new();
            for b in &page.boxes {
                texts(b, &mut out);
            }
            out
        })
        .collect()
}

/// 指定のテキストを含むボックスの border box を、全ページから探す。
fn find_box_rect(html_src: &str, needle: &str) -> (usize, sghtmltopdf_core::layout::Rect) {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);

    fn text_of(b: &LaidOutBox) -> String {
        let mut out = String::new();
        fn walk(b: &LaidOutBox, out: &mut String) {
            if let LaidOutContent::Inline(lines) = &b.content {
                for l in lines {
                    for r in &l.runs {
                        out.push_str(&r.text);
                    }
                }
            }
            if let LaidOutContent::Blocks(children) = &b.content {
                for c in children {
                    walk(c, out);
                }
            }
        }
        walk(b, &mut out);
        out
    }
    for (i, page) in pages.iter().enumerate() {
        for b in &page.boxes {
            if text_of(b).contains(needle) {
                return (i, b.layout.border_box());
            }
        }
    }
    panic!("no box containing {needle:?}");
}

#[test]
fn a_fixed_element_repeats_on_every_page() {
    let long = "<div style=\"height: 900px;\">tall</div>";
    let html_src = format!(
        "<body><p>one</p>{long}<p>two</p>\
         <div style=\"position: fixed; top: 100px; left: 50px;\">WATERMARK</div></body>"
    );
    let per_page = pages_of(&html_src);
    assert!(
        per_page.len() >= 2,
        "the document should span several pages"
    );
    for (i, texts) in per_page.iter().enumerate() {
        assert!(
            texts.iter().any(|t| t.contains("WATERMARK")),
            "page {i} should contain the fixed watermark, got {texts:?}"
        );
    }
}

#[test]
fn an_absolute_element_only_appears_once() {
    let long = "<div style=\"height: 900px;\">tall</div>";
    let html_src = format!(
        "<body><p>one</p>{long}<p>two</p>\
         <div style=\"position: absolute; top: 10px; left: 10px;\">ABS</div></body>"
    );
    let per_page = pages_of(&html_src);
    let count: usize = per_page
        .iter()
        .map(|texts| texts.iter().filter(|t| t.contains("ABS")).count())
        .sum();
    assert_eq!(count, 1, "an absolute element must not repeat");
    // positioned祖先が無いので initial containing block = 最初のページ。
    assert!(per_page[0].iter().any(|t| t.contains("ABS")));
}

#[test]
fn an_absolute_child_is_placed_relative_to_its_positioned_ancestor() {
    // カード(relative, ページ幅いっぱい)の右上に absolute のバッジ。祖先の
    // padding box基準で right 配置されるので、バッジはページの右寄りに来る。
    let content_width = PageSettings::default().content_width();
    let with_right = r#"<body>
        <div style="position: relative; margin: 20px; padding: 10px; height: 100px;">
          <span style="position: absolute; top: 5px; right: 5px;">BADGE</span>
          card body
        </div></body>"#;
    let (_, badge_right) = find_box_rect(with_right, "BADGE");
    assert!(
        badge_right.x > content_width * 0.5,
        "a right-anchored badge should sit in the right half: x={} of {content_width}",
        badge_right.x
    );

    // 同じ祖先で left 配置なら左寄りになる(祖先の左端 = margin 20 + padding 10)。
    let with_left = with_right.replace("right: 5px", "left: 5px");
    let (_, badge_left) = find_box_rect(&with_left, "BADGE");
    assert!(
        badge_left.x < content_width * 0.3,
        "a left-anchored badge should sit near the left: x={}",
        badge_left.x
    );
    assert!(badge_right.x > badge_left.x);
}

#[test]
fn a_left_absolute_sits_at_the_left_and_a_right_absolute_at_the_right() {
    let html_src = r#"<body><div style="position: relative; height: 200px; margin: 0;">
        <span style="position: absolute; left: 0;">L</span>
        <span style="position: absolute; right: 0;">R</span>
    </div></body>"#;
    let (_, l) = find_box_rect(html_src, "L");
    let (_, r) = find_box_rect(html_src, "R");
    assert!(r.x > l.x, "the right-anchored box must be further right");
}

#[test]
fn a_fixed_footer_uses_bottom() {
    // fixed は cb 高さ(ページ高さ)が確定するので bottom が効く。
    let html_src = r#"<body><p>content</p>
        <div style="position: fixed; bottom: 20px; left: 40px;">FOOTER</div></body>"#;
    let (_, footer) = find_box_rect(html_src, "FOOTER");
    let page_height = PageSettings::default().size.height;
    // フッタはページ下部にある。
    assert!(
        footer.y > page_height * 0.7,
        "footer at y={} should be near the bottom of {page_height}",
        footer.y
    );
}

#[test]
fn absolute_elements_do_not_take_space_in_the_normal_flow() {
    // absolute はフローから外れるので、後続の通常フロー要素の位置に影響しない。
    let without = find_box_rect("<body><p>A</p><p>B</p></body>", "B").1;
    let with = find_box_rect(
        r#"<body><p>A</p><div style="position: absolute; top: 0;">X</div><p>B</p></body>"#,
        "B",
    )
    .1;
    assert!(
        (with.y - without.y).abs() < 0.5,
        "the absolute element must not push B down: {} vs {}",
        with.y,
        without.y
    );
}

#[test]
fn a_document_with_absolute_and_fixed_encodes_to_a_valid_pdf_in_batch_mode() {
    let html_src = r#"<html><body>
        <div style="position: fixed; top: 300px; left: 200px;">COPY</div>
        <div style="position: relative; height: 100px;">
          <span style="position: absolute; top: 0; right: 0;">TAG</span>
          body
        </div>
      </body></html>"#;
    let options = EngineOptions {
        mode: Mode::Batch,
        fonts: vec![FontSpec {
            path: std::path::PathBuf::from(FONT_PATH),
            index: 0,
        }],
        ..EngineOptions::default()
    };
    let mut engine = Engine::new(options, MemorySink::new());
    engine.feed(html_src.as_bytes()).unwrap();
    let bytes = engine.finish().unwrap();

    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.windows(5).any(|w| w == b"%%EOF"));
}
