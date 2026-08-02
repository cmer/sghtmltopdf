//! 通常モード(`Mode::Batch`)とストリーミングモード(`Mode::Streaming`)の
//! ピークメモリ・処理時間を測る。ドキュメントサイトの「メモリと処理時間」の表と
//! 同じ条件(HTMLの形・フォント明示・試行回数)を測り、そのまま貼れる
//! マークダウンの表として出す。
//!
//! 実行: `cargo run --release --example mem_bench`
//!
//! 測るのはプロセスのピークRSS(Linuxの`/proc/self/status`の`VmHWM`)なので、
//! 1プロセスで両モードを回すと先に走ったほうの山が残って比較にならない。
//! そのため親プロセスが条件ごとに自分自身を再実行し、子プロセスが自分の
//! ピークRSSを報告する構成にしている(`SGHTMLTOPDF_BENCH_CASE`が
//! 設定されていれば子として振る舞う)。
//!
//! 数字は環境に強く依存する。ドキュメントに載せるときは測定に使った
//! マシンとビルド種別も一緒に書くこと。

use std::env;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use sghtmltopdf_core::engine::{Engine, EngineOptions, FontSpec, Mode};
use sghtmltopdf_core::sink::FileSink;

/// 測る文書サイズ(`<p>`要素の数)。
const ELEMENT_COUNTS: &[usize] = &[1_000, 5_000, 20_000, 60_000];

/// 条件ごとの試行回数。良いほうの値を採る。
const RUNS: usize = 2;

/// 1MiBに満たない端数を丸めるための除数。
const MIB: f64 = 1024.0;

fn main() {
    // 子プロセスとして呼ばれた場合は1条件だけ測って結果を1行で返す。
    if let Ok(case) = env::var("SGHTMLTOPDF_BENCH_CASE") {
        run_one_case(&case);
        return;
    }

    let exe = env::current_exe().expect("実行ファイルのパスを取得できません");
    println!("| 要素数 | HTMLサイズ | 通常モード | --streaming |");
    println!("|---|---|---|---|");
    for &count in ELEMENT_COUNTS {
        let html_bytes = build_html(count).len();
        let batch = best_of(&exe, count, Mode::Batch);
        let streaming = best_of(&exe, count, Mode::Streaming);
        println!(
            "| {} | {} | {} | {} |",
            group_digits(count),
            format_bytes(html_bytes),
            batch,
            streaming,
        );
    }
}

/// 同じ条件を[`RUNS`]回測り、ピークRSSと処理時間それぞれの最小値を返す。
fn best_of(exe: &Path, count: usize, mode: Mode) -> String {
    let mut best_rss = f64::MAX;
    let mut best_secs = f64::MAX;
    for _ in 0..RUNS {
        let (rss_kib, secs) = spawn_case(exe, count, mode);
        best_rss = best_rss.min(rss_kib);
        best_secs = best_secs.min(secs);
    }
    format!("{:.0}MB / {:.2}秒", best_rss / MIB, best_secs)
}

/// 自分自身を子プロセスとして起動し、`(ピークRSS[KiB], 秒)`を受け取る。
fn spawn_case(exe: &Path, count: usize, mode: Mode) -> (f64, f64) {
    let mode_name = match mode {
        Mode::Batch => "batch",
        Mode::Streaming => "streaming",
    };
    let output = Command::new(exe)
        .env("SGHTMLTOPDF_BENCH_CASE", format!("{count}:{mode_name}"))
        .output()
        .expect("子プロセスを起動できません");
    if !output.status.success() {
        panic!(
            "子プロセスが失敗しました: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let mut parts = line.split_whitespace();
    let rss = parts.next().and_then(|v| v.parse().ok());
    let secs = parts.next().and_then(|v| v.parse().ok());
    match (rss, secs) {
        (Some(rss), Some(secs)) => (rss, secs),
        _ => panic!("子プロセスの出力を解釈できません: {line:?}"),
    }
}

/// 子プロセス側。1条件だけ変換し、「ピークRSS[KiB] 秒」を標準出力へ書く。
fn run_one_case(case: &str) {
    let (count, mode_name) = case.split_once(':').expect("条件の形式が不正です");
    let count: usize = count.parse().expect("要素数を解釈できません");
    let mode = match mode_name {
        "batch" => Mode::Batch,
        "streaming" => Mode::Streaming,
        other => panic!("未知のモード: {other}"),
    };

    let html = build_html(count);
    let started = Instant::now();
    convert(&html, mode);
    let secs = started.elapsed().as_secs_f64();

    println!("{} {secs}", peak_rss_kib());
}

/// 変換を1回走らせる。
///
/// 出力先は一時ファイル([`FileSink`]、CLIと同じ)にする。[`MemorySink`]だと
/// PDF全体がメモリに残り、ストリーミングモードの数字がPDFのサイズぶん
/// 押し上げられてしまうため。
fn convert(html: &str, mode: Mode) {
    let options = EngineOptions {
        mode,
        // システムフォントの探索結果で数字がぶれないよう、フォントを明示する。
        fonts: vec![FontSpec {
            path: font_path(),
            index: 0,
        }],
        ..EngineOptions::default()
    };
    let out_path =
        env::temp_dir().join(format!("sghtmltopdf-mem-bench-{}.pdf", std::process::id()));
    let sink = FileSink::create(&out_path).expect("出力先を作れません");
    let mut engine = Engine::new(options, sink);
    // 実際の利用に近づけるため、64KiBずつ流し込む。
    for chunk in html.as_bytes().chunks(64 * 1024) {
        engine.feed(chunk).expect("feedに失敗しました");
    }
    engine.finish().expect("finishに失敗しました");
    let _ = std::fs::remove_file(&out_path);
}

/// 高さ60pxの`<p>`を`count`個並べただけのHTML。
fn build_html(count: usize) -> String {
    let mut html = String::with_capacity(count * 48);
    html.push_str("<html><head><style>p { height: 60px; margin: 0; }</style></head><body>");
    for i in 0..count {
        let _ = write!(html, "<p>paragraph {i} lorem ipsum dolor sit amet</p>");
    }
    html.push_str("</body></html>");
    html
}

fn font_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/DejaVuSans.ttf")
}

/// このプロセスがこれまでに使ったRSSの最大値(KiB)。
///
/// Linux以外では`VmHWM`が無いため0を返す(表は処理時間だけが意味を持つ)。
fn peak_rss_kib() -> f64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0.0;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|kib| kib.parse().ok())
        .unwrap_or(0.0)
}

fn format_bytes(bytes: usize) -> String {
    let kib = bytes as f64 / 1024.0;
    if kib < 1024.0 {
        format!("{kib:.0}KB")
    } else {
        format!("{:.1}MB", kib / 1024.0)
    }
}

/// 1000区切りのカンマを入れる(表の読みやすさのため)。
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
