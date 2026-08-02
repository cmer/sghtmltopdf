//! ヒープの割り当てを呼び出し元ごとに集計する。ピークメモリの犯人探し用。
//!
//! 実行: `cargo run --release --example heap_profile [要素数] [table]`
//!
//! `dhat`(dev-dependency)がグローバルアロケータを差し替え、終了時に
//! `dhat-heap.json`を書き出す。同梱の`heap_report.py`に食わせると、
//! ピーク時点でメモリを持っていた箇所を多い順に表示できる。
//!
//! `phase_bench`が「どの段が重いか」を見るのに対し、こちらは
//! 「どのコードが確保しているか」を見る。dhatは全割り当てを記録するので
//! 実行は数倍遅くなる。時間の計測には使わないこと。

use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::path::Path;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{paginate_document, PageSettings};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let profiler = dhat::Profiler::new_heap();

    let count: usize = env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let table_mode = env::args().nth(2).is_some_and(|v| v == "table");
    let html_src = build_html(count, table_mode);

    let fonts = load_fonts();
    let settings = PageSettings::default();
    let dom = html::parse(html_src.as_bytes());
    let author = parse_stylesheet(if table_mode {
        "table { border-collapse: collapse; } th, td { border: 1px solid #999999; padding: 4px 6px; }"
    } else {
        "p { height: 60px; margin: 0; }"
    });
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &author);
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

    println!(
        "ページ数 {} / PDF {:.1}KB",
        pages.len(),
        bytes.len() as f64 / 1024.0
    );
    // ここでdrop すると dhat-heap.json が書かれる。
    drop(profiler);
    println!("dhat-heap.json を書き出しました");
}

fn build_html(count: usize, table_mode: bool) -> String {
    let mut html = String::with_capacity(count * 120);
    html.push_str("<html><head></head><body>");
    if table_mode {
        html.push_str("<table>");
        for i in 0..count {
            let _ = write!(
                html,
                "<tr><td>{i}</td><td>Item {i} description text</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                i % 97 + 1,
                (i % 97 + 1) * 120,
                (i % 97 + 1) * 360
            );
        }
        html.push_str("</table>");
    } else {
        for i in 0..count {
            let _ = write!(html, "<p>paragraph {i} lorem ipsum dolor sit amet</p>");
        }
    }
    html.push_str("</body></html>");
    html
}

fn load_fonts() -> FontCollection {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/DejaVuSans.ttf");
    let data = std::fs::read(path).expect("フォントを読めません");
    let font = Font::from_bytes(data, 0).expect("フォントを解釈できません");
    FontCollection::new(vec![font])
}
