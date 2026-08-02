//! パイプラインの各段(パース → スタイル → box tree → レイアウト →
//! ページ分割 → PDFエンコード)にかかる時間を測って内訳を出す。
//!
//! 実行: `cargo run --release --example phase_bench [要素数]`
//!
//! `mem_bench`が全体の数字を出すのに対し、こちらは「どの段が重いか」を見る。

use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use sghtmltopdf_core::fonts::{Font, FontCollection};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{build_box_tree, layout_document, paginate_document, PageSettings};
use sghtmltopdf_core::pdf::encode_pdf;
use sghtmltopdf_core::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

/// 割り当てを数えるだけのアロケータ。どのサイズ帯がメモリを占めているかを見る。
struct CountingAlloc;

static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// サイズ帯(2の冪)ごとの、現在生きている割り当てのバイト数。
static BUCKETS: [std::sync::atomic::AtomicUsize; 24] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 24];
/// ピークを更新した瞬間のサイズ帯別内訳(どの構造がピークを作っているかを見る)。
static PEAK_BUCKETS: [std::sync::atomic::AtomicUsize; 24] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 24];
/// サイズ帯ごとの、現在生きている割り当ての件数。
static COUNTS: [std::sync::atomic::AtomicUsize; 24] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 24];
/// ピーク時点の件数。
static PEAK_COUNTS: [std::sync::atomic::AtomicUsize; 24] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 24];
/// 次に内訳を控える生存量のしきい値。
static NEXT_SNAPSHOT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn bucket_of(size: usize) -> usize {
    ((usize::BITS - size.leading_zeros()) as usize).min(23)
}

unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { std::alloc::System.alloc(layout) };
        if !ptr.is_null() {
            let now =
                LIVE.fetch_add(layout.size(), std::sync::atomic::Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, std::sync::atomic::Ordering::Relaxed);
            BUCKETS[bucket_of(layout.size())]
                .fetch_add(layout.size(), std::sync::atomic::Ordering::Relaxed);
            COUNTS[bucket_of(layout.size())].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // 生存量が8MB伸びるごとに、その時点のサイズ帯別内訳を控える。
            if now > NEXT_SNAPSHOT.load(std::sync::atomic::Ordering::Relaxed) {
                NEXT_SNAPSHOT.store(now + 8 * 1024 * 1024, std::sync::atomic::Ordering::Relaxed);
                for (dst, src) in PEAK_BUCKETS.iter().zip(BUCKETS.iter()) {
                    dst.store(
                        src.load(std::sync::atomic::Ordering::Relaxed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                for (dst, src) in PEAK_COUNTS.iter().zip(COUNTS.iter()) {
                    dst.store(
                        src.load(std::sync::atomic::Ordering::Relaxed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        LIVE.fetch_sub(layout.size(), std::sync::atomic::Ordering::Relaxed);
        BUCKETS[bucket_of(layout.size())]
            .fetch_sub(layout.size(), std::sync::atomic::Ordering::Relaxed);
        COUNTS[bucket_of(layout.size())].fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// ピークをこの時点の生存量まで戻す(段ごとのピークを見るため)。
fn reset_peak() {
    use std::sync::atomic::Ordering::Relaxed;
    let live = LIVE.load(Relaxed);
    PEAK.store(live, Relaxed);
    NEXT_SNAPSHOT.store(live, Relaxed);
    for b in PEAK_BUCKETS.iter() {
        b.store(0, Relaxed);
    }
}

/// 生きている割り当てをサイズ帯ごとに表示する。
fn dump_live_allocations(label: &str) {
    use std::sync::atomic::Ordering::Relaxed;
    let live = LIVE.load(Relaxed) as f64 / 1024.0 / 1024.0;
    println!(
        "{label}: 生存 {live:.0}MB (ピーク {:.0}MB)",
        PEAK.load(Relaxed) as f64 / 1024.0 / 1024.0
    );
    println!("  ピーク時点の内訳:");
    for (i, b) in PEAK_BUCKETS.iter().enumerate() {
        let mb = b.load(Relaxed) as f64 / 1024.0 / 1024.0;
        let n = PEAK_COUNTS[i].load(Relaxed);
        if mb >= 5.0 {
            println!(
                "    ~{:>8}B  {:>6.0}MB  {:>9}件  平均{:>6.0}B",
                1usize << i,
                mb,
                n,
                if n > 0 {
                    b.load(Relaxed) as f64 / n as f64
                } else {
                    0.0
                }
            );
        }
    }
}

fn main() {
    {
        use sghtmltopdf_core::layout::{LaidOutBox, LayoutBox};
        use sghtmltopdf_core::style::ComputedStyle;
        use std::mem::size_of;
        println!(
            "型サイズ: ComputedStyle {}B / LayoutBox {}B / LaidOutBox {}B",
            size_of::<ComputedStyle>(),
            size_of::<LayoutBox>(),
            size_of::<LaidOutBox>(),
        );
        use sghtmltopdf_core::fonts::ShapedGlyph;
        use sghtmltopdf_core::layout::{LaidOutContent, Layout};
        use sghtmltopdf_core::layout::{LineBox, TextRun};
        println!(
            "          Layout {}B / LaidOutContent {}B / Option<LineBox> {}B",
            size_of::<Layout>(),
            size_of::<LaidOutContent>(),
            size_of::<Option<LineBox>>(),
        );
        println!(
            "          LineBox {}B / TextRun {}B / ShapedGlyph {}B",
            size_of::<LineBox>(),
            size_of::<TextRun>(),
            size_of::<ShapedGlyph>(),
        );
    }
    println!("開始時RSS {:.0}MB", rss_mb());
    let count: usize = env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let html_src = build_html(count);
    println!(
        "要素数 {count} / HTML {:.1}KB",
        html_src.len() as f64 / 1024.0
    );

    let fonts = load_fonts();
    let settings = PageSettings::default();
    let mut phases: Vec<(&str, f64, f64)> = Vec::new();

    reset_peak();
    let t = Instant::now();
    let dom = html::parse(html_src.as_bytes());
    phases.push(("HTMLパース", t.elapsed().as_secs_f64(), rss_mb()));
    dump_live_allocations("HTMLパース");
    reset_peak();

    let ua = user_agent_stylesheet();
    let author = parse_stylesheet("p { height: 60px; margin: 0; }");

    let t = Instant::now();
    let styles = compute_styles(&dom, &ua, &author);
    phases.push(("スタイル計算", t.elapsed().as_secs_f64(), rss_mb()));
    dump_live_allocations("スタイル計算");
    reset_peak();

    let t = Instant::now();
    let tree = build_box_tree(&dom, &styles);
    phases.push(("box tree構築", t.elapsed().as_secs_f64(), rss_mb()));
    dump_live_allocations("box tree構築");
    reset_peak();

    let t = Instant::now();
    let laid = layout_document(&tree, &styles, &fonts, settings.content_width());
    phases.push(("レイアウト", t.elapsed().as_secs_f64(), rss_mb()));
    dump_live_allocations("レイアウト");
    reset_peak();
    let mut c = Counts::default();
    count_boxes(&laid, &mut c);
    println!(
        "レイアウト結果: box {} / 行 {} / ラン {} / グリフ {} (1段落あたり ラン{:.1} グリフ{:.1})",
        c.boxes,
        c.lines,
        c.runs,
        c.glyphs,
        c.runs as f64 / count as f64,
        c.glyphs as f64 / count as f64
    );
    {
        use sghtmltopdf_core::fonts::ShapedGlyph;
        use sghtmltopdf_core::layout::{LaidOutBox, LineBox, TextRun};
        use std::mem::size_of;
        let real = c.boxes * size_of::<LaidOutBox>()
            + c.lines * size_of::<LineBox>()
            + c.runs * size_of::<TextRun>()
            + c.glyphs * size_of::<ShapedGlyph>();
        println!(
            "  実データ {:.0}MB / Vecの余剰容量 {:.0}MB",
            real as f64 / 1024.0 / 1024.0,
            c.slack as f64 / 1024.0 / 1024.0
        );
    }
    dump_live_allocations("レイアウト後");
    drop(laid);

    // ページ分割はレイアウトを内部でやり直すため、上の計測とは別に丸ごと測る。
    let t = Instant::now();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    phases.push((
        "ページ分割(レイアウト込み)",
        t.elapsed().as_secs_f64(),
        rss_mb(),
    ));
    dump_live_allocations("ページ分割後");

    let t = Instant::now();
    let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
    phases.push(("PDFエンコード", t.elapsed().as_secs_f64(), rss_mb()));

    println!(
        "ページ数 {} / PDF {:.1}KB\n",
        pages.len(),
        bytes.len() as f64 / 1024.0
    );

    // 「パース + スタイル + ページ分割 + エンコード」が実際の総処理時間にあたる。
    let total: f64 = phases
        .iter()
        .filter(|(name, _, _)| *name != "レイアウト" && *name != "box tree構築")
        .map(|(_, secs, _)| secs)
        .sum();
    println!("{:<28} {:>8}  {:>6}  {:>10}", "段", "秒", "割合", "常駐RSS");
    for (name, secs, rss) in &phases {
        let share = if *name == "レイアウト" || *name == "box tree構築" {
            "(内訳)".to_string()
        } else {
            format!("{:.0}%", secs / total * 100.0)
        };
        println!("{name:<28} {secs:>8.2}  {share:>6}  {rss:>8.0}MB");
    }
    println!("{:<28} {total:>8.2}", "合計(実処理)");
}

fn build_html(count: usize) -> String {
    // 第2引数に`empty`を渡すとテキストを持たない`<p>`にする。
    // テキスト処理(シェイピング・行分割)の寄与を切り分けるため。
    let mode = env::args().nth(2).unwrap_or_default();
    let empty = mode == "empty";
    if mode == "table" {
        let mut html = String::with_capacity(count * 120);
        html.push_str(
            "<html><head><style>table { border-collapse: collapse; } \
             th, td { border: 1px solid #999999; padding: 4px 6px; }</style></head><body><table>",
        );
        for i in 0..count {
            let _ = write!(
                html,
                "<tr><td>{i}</td><td>Item {i} description text</td><td>{}</td>\
                 <td>{}</td><td>{}</td></tr>",
                i % 97 + 1,
                (i % 97 + 1) * 120,
                (i % 97 + 1) * 360
            );
        }
        html.push_str("</table></body></html>");
        return html;
    }
    let mut html = String::with_capacity(count * 48);
    html.push_str("<html><head><style>p { height: 60px; margin: 0; }</style></head><body>");
    for i in 0..count {
        if empty {
            html.push_str("<p></p>");
        } else {
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

/// 現在の常駐メモリ(MB)。Linux以外では0。
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next())
                .and_then(|kib| kib.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
        / 1024.0
}

#[derive(Default)]
struct Counts {
    boxes: usize,
    lines: usize,
    runs: usize,
    glyphs: usize,
    slack: usize,
}

/// レイアウト結果を走査して、保持している要素数を数える。
fn count_boxes(b: &sghtmltopdf_core::layout::LaidOutBox, c: &mut Counts) {
    use sghtmltopdf_core::layout::LaidOutContent;
    c.boxes += 1;
    match &b.content {
        LaidOutContent::Blocks(children) => {
            for child in children {
                count_boxes(child, c);
            }
        }
        LaidOutContent::Table(table) => {
            for row in &table.rows {
                for cell in &row.cells {
                    count_boxes(cell, c);
                }
            }
        }
        LaidOutContent::Inline(lines) => {
            c.slack += (lines.capacity() - lines.len())
                * std::mem::size_of::<sghtmltopdf_core::layout::LineBox>();
            for line in lines {
                c.lines += 1;
                c.runs += line.runs.len();
                c.slack += (line.runs.capacity() - line.runs.len())
                    * std::mem::size_of::<sghtmltopdf_core::layout::TextRun>();
                for run in &line.runs {
                    c.glyphs += run.glyphs.len();
                    c.slack += (run.glyphs.capacity() - run.glyphs.len())
                        * std::mem::size_of::<sghtmltopdf_core::fonts::ShapedGlyph>();
                    c.slack += run.text.capacity() - run.text.len();
                }
            }
        }
        _ => {}
    }
}
