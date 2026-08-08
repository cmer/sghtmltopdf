//! sghtmltopdf・wkhtmltopdf・ヘッドレスChromeのピークメモリと処理時間を
//! 同条件で比較する。ドキュメントサイトのパフォーマンス比較の表はこれで取っている。
//!
//! 実行:
//!
//! ```text
//! cargo build --release                      # 比較対象のCLIを先に作る
//! cargo run --release --example compare_engines
//! ```
//!
//! 見つからないエンジンは飛ばす。場所は環境変数でも指定できる。
//!
//! * `WKHTMLTOPDF` — 公式配布はパッケージのみで単体バイナリが無いため、
//!   インストールせずに使うならdebを展開するのが手軽:
//!   `dpkg-deb -x wkhtmltox_0.12.6.1-3.jammy_amd64.deb /tmp/wk`
//!   (実行には`xfonts-base`/`xfonts-75dpi`が要る)
//! * `CHROME` — 未指定なら`google-chrome`を`PATH`から探す
//!
//! # 比較を成立させるために揃えていること
//!
//! * 用紙と余白は`@page`で指定する。3者とも同じCSSで同じ幾何になる
//!   (wkhtmltopdfだけは`@page`の`size`を見ないので、同じ値をCLIでも渡す)
//! * 同じフォントファイルを`@font-face`で参照させる
//! * wkhtmltopdfはCSSのpxを1/72インチとして扱うので、[`ZOOM`]で1px = 1/96
//!   インチへ寄せる。sghtmltopdfとChromeは1/96インチなので補正は要らない
//! * JavaScriptは実行しない
//!
//! それでもページ数は完全一致しない。表にページ数も出しているので、比較が
//! 成立している範囲かは都度確認すること。
//!
//! # メモリの測り方
//!
//! Chromeはブラウザ・レンダラ・GPUと複数プロセスに分かれるため、起動した
//! プロセスだけを見ると実態の半分以下になる(実測で246MB対567MB)。
//! そこで[`tree_pss_kib`]がプロセスツリー全体の`Pss`合計をサンプリングし、
//! その最大値を採る。3者とも同じ方法で測っている。
//!
//! サンプリングなので[`SAMPLE_INTERVAL`]より短いスパイクは取り逃す可能性が
//! ある。また測るのはブラウザの起動を含めた1回の変換で、常駐させて使い回す
//! 運用(CDP経由でプールする等)の数値ではない。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 測る文書の規模。
const PARAGRAPH_COUNTS: &[usize] = &[5_000, 20_000, 60_000];
const TABLE_COUNTS: &[usize] = &[5_000, 20_000];

/// 条件ごとの試行回数。良いほうの値を採る。
const RUNS: usize = 2;

/// メモリのサンプリング間隔。
const SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

/// wkhtmltopdfのpx解釈(1px = 1/72インチ)を1px = 1/96インチへ寄せる倍率。
const ZOOM: &str = "1.3333333";

/// 用紙と余白。`@page`とwkhtmltopdfのCLIの両方へ同じ値を書く。
const PAGE_SIZE: &str = "A4";
const MARGIN: &str = "10mm";

fn main() {
    let sghtmltopdf = release_binary();
    if !sghtmltopdf.exists() {
        eprintln!(
            "{} がありません。先に cargo build --release を実行してください",
            sghtmltopdf.display()
        );
        std::process::exit(1);
    }
    let wkhtmltopdf = find_binary("WKHTMLTOPDF", "wkhtmltopdf");
    let chrome = find_binary("CHROME", "google-chrome");
    for (name, found) in [("wkhtmltopdf", &wkhtmltopdf), ("chrome", &chrome)] {
        if found.is_none() {
            eprintln!("{name} が見つからないため、その列は飛ばします");
        }
    }

    let work = env::temp_dir().join("sghtmltopdf-compare");
    std::fs::create_dir_all(&work).expect("作業ディレクトリを作れません");
    std::fs::copy(font_path(), work.join("font.ttf")).expect("フォントを複製できません");

    for (title, header, counts, table_mode) in [
        ("### 段落が主体の文書", "要素数", PARAGRAPH_COUNTS, false),
        ("### 表が主体の帳票", "行数", TABLE_COUNTS, true),
    ] {
        let mut columns = vec!["sghtmltopdf", "sghtmltopdf（ストリーミング）"];
        if wkhtmltopdf.is_some() {
            columns.push("wkhtmltopdf");
        }
        if chrome.is_some() {
            columns.push("ヘッドレスChrome");
        }
        println!("\n{title}\n");
        println!("| {header} | {} | ページ数 |", columns.join(" | "));
        println!("|{}|", "---|".repeat(columns.len() + 2));

        for &count in counts {
            let name = if table_mode { "table" } else { "para" };
            let html = work.join(format!("{name}{count}.html"));
            std::fs::write(&html, build_html(count, table_mode)).expect("HTMLを書けません");

            let mut cells = Vec::new();
            let mut pages = Vec::new();
            // 変換できなかったエンジンは、そこだけセルを`-`にして続ける
            // (ストリーミングで使えないCSSを含む文書など)。
            let mut measure =
                |label: &str, command: &dyn Fn(&Path) -> Command, count_it: bool| match best_of(
                    &work, command,
                ) {
                    Some((cell, page_count)) => {
                        cells.push(cell);
                        if count_it {
                            pages.push(page_count.to_string());
                        }
                    }
                    None => {
                        eprintln!("{name}{count}: {label} は変換に失敗したので `-` にします");
                        cells.push("-".to_string());
                        if count_it {
                            pages.push("-".to_string());
                        }
                    }
                };

            measure(
                "sghtmltopdf",
                &|out| sg_command(&sghtmltopdf, &html, out, false),
                true,
            );
            measure(
                "sghtmltopdf（ストリーミング）",
                &|out| sg_command(&sghtmltopdf, &html, out, true),
                false,
            );
            if let Some(wk) = &wkhtmltopdf {
                measure("wkhtmltopdf", &|out| wk_command(wk, &html, out), true);
            }
            if let Some(chrome) = &chrome {
                measure(
                    "ヘッドレスChrome",
                    &|out| chrome_command(chrome, &html, out),
                    true,
                );
            }
            println!(
                "| {} | {} | {} |",
                group_digits(count),
                cells.join(" | "),
                pages.join(" / ")
            );
        }
    }
}

/// `build`が返すコマンドを[`RUNS`]回動かし、「ピークメモリ / 処理時間」と
/// 生成されたPDFのページ数を返す。
///
/// 1回でも変換に失敗したら`None`。失敗した回の数値は当てにならないので、
/// 成功した回だけで平均を取るようなことはしない。
fn best_of(work: &Path, build: &dyn Fn(&Path) -> Command) -> Option<(String, usize)> {
    let out = work.join("out.pdf");
    let mut best_kib = f64::MAX;
    let mut best_secs = f64::MAX;
    for _ in 0..RUNS {
        let _ = std::fs::remove_file(&out);
        let (kib, secs) = run_and_measure(build(&out))?;
        best_kib = best_kib.min(kib);
        best_secs = best_secs.min(secs);
    }
    Some((
        format!("{:.0}MB / {:.2}秒", best_kib / 1024.0, best_secs),
        count_pages(&out),
    ))
}

/// コマンドを動かし、`(ピークPSS[KiB], 秒)`を返す。変換が失敗したら`None`。
fn run_and_measure(mut command: Command) -> Option<(f64, f64)> {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("変換コマンドを起動できません");

    let mut peak = 0;
    loop {
        match child.try_wait().expect("子プロセスの状態を取得できません") {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            None => {
                peak = peak.max(tree_pss_kib(child.id()));
                std::thread::sleep(SAMPLE_INTERVAL);
            }
        }
    }
    Some((peak as f64, started.elapsed().as_secs_f64()))
}

/// `root`とその子孫プロセスの`Pss`の合計(KiB)。
///
/// `Pss`(proportional set size)は共有ページを共有しているプロセス数で割って
/// 数えるので、マルチプロセスのブラウザでも二重計上にならない。
fn tree_pss_kib(root: u32) -> u64 {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            continue;
        };
        if let Some(ppid) = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .and_then(|value| value.trim().parse::<u32>().ok())
        {
            children.entry(ppid).or_default().push(pid);
        }
    }

    let mut total = 0;
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Ok(rollup) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
            total += rollup
                .lines()
                .find_map(|line| line.strip_prefix("Pss:"))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|kib| kib.parse::<u64>().ok())
                .unwrap_or(0);
        }
        stack.extend(children.get(&pid).into_iter().flatten());
    }
    total
}

fn sg_command(sghtmltopdf: &Path, html: &Path, out: &Path, streaming: bool) -> Command {
    let mut command = Command::new(sghtmltopdf);
    command
        .arg(html)
        .arg("-o")
        .arg(out)
        .arg("--enable-local-file-access")
        .arg("-q");
    if streaming {
        command.arg("--streaming");
    }
    command
}

fn wk_command(wkhtmltopdf: &Path, html: &Path, out: &Path) -> Command {
    let mut command = Command::new(wkhtmltopdf);
    command
        .arg("-q")
        .arg("--disable-javascript")
        // `@font-face`のローカルフォントを読ませるために要る。
        .arg("--enable-local-file-access")
        .arg("--zoom")
        .arg(ZOOM)
        // wkhtmltopdfは`@page`の`size`/`margin`を見ないのでCLIでも渡す。
        .arg("--page-size")
        .arg(PAGE_SIZE)
        .args(["-T", MARGIN, "-B", MARGIN, "-L", MARGIN, "-R", MARGIN])
        .arg(html)
        .arg(out);
    command
}

fn chrome_command(chrome: &Path, html: &Path, out: &Path) -> Command {
    let mut command = Command::new(chrome);
    command
        .arg("--headless")
        // コンテナやWSLで動かすために要る(プロセス構成は変わる)。
        .arg("--no-sandbox")
        .arg("--disable-gpu")
        .arg("--disable-extensions")
        .arg("--no-first-run")
        .arg(format!("--print-to-pdf={}", out.display()))
        .arg(html);
    command
}

/// PDF内の`/Type /Page`の個数。3者が同じ量を処理したかの確認用。
fn count_pages(pdf: &Path) -> usize {
    let bytes = std::fs::read(pdf).unwrap_or_default();
    let mut count = 0;
    for (i, _) in bytes.windows(5).enumerate().filter(|(_, w)| *w == b"/Type") {
        // `/Type`と`/Page`の間の空白の有無は書き手によって違う。
        let mut j = i + 5;
        while matches!(bytes.get(j), Some(b' ' | b'\n' | b'\r')) {
            j += 1;
        }
        let rest = &bytes[j..];
        if rest.starts_with(b"/Page") && !rest.starts_with(b"/Pages") {
            count += 1;
        }
    }
    count
}

/// 比較用のHTML。用紙は`@page`で、寸法はpxで書く(3者が解釈できる単位のため)。
fn build_html(count: usize, table_mode: bool) -> String {
    let mut html = String::with_capacity(count * 120);
    let _ = write!(
        html,
        "<html><head><meta charset=\"utf-8\"><style>\
         @page {{ size: {PAGE_SIZE}; margin: {MARGIN}; }}\
         @font-face {{ font-family: 'BenchSans'; src: url('font.ttf') format('truetype'); }}\
         html, body {{ margin: 0; padding: 0; }}\
         body {{ font-family: 'BenchSans'; font-size: 12px; line-height: 1.4; }}"
    );
    if table_mode {
        html.push_str(
            "table { width: 100%; border-collapse: collapse; }\
             th, td { border: 1px solid #999999; padding: 4px 6px; }",
        );
        html.push_str("</style></head><body><table>");
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
        html.push_str("p { height: 60px; margin: 0; }</style></head><body>");
        for i in 0..count {
            let _ = write!(html, "<p>paragraph {i} lorem ipsum dolor sit amet</p>");
        }
    }
    html.push_str("</body></html>");
    html
}

/// `env_var`で指定された場所、無ければ`PATH`から探す。
fn find_binary(env_var: &str, name: &str) -> Option<PathBuf> {
    if let Ok(path) = env::var(env_var) {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    let found = Command::new("which").arg(name).output().ok()?;
    found
        .status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&found.stdout).trim()))
}

/// `cargo build --release`が置くCLIバイナリ。
fn release_binary() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/release")
        .join("sghtmltopdf")
}

fn font_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/DejaVuSans.ttf")
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
