//! sghtmltopdf CLI: HTMLファイルを一括変換してPDFを出力する。
//!
//! M1では静的HTML一括変換(ストリーミングなし)のみ対応。フォントはローカル
//! ファイルパス指定の最小実装(`--font`必須)で、システムフォント探索・
//! `@font-face`によるwebfont解決は将来のマイルストーンで対応する。

use std::path::PathBuf;
use std::process::ExitCode;

use sghtmltopdf_core::fonts::Font;
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{paginate_document, PageSettings};
use sghtmltopdf_core::pdf::write_document;
use sghtmltopdf_core::sink::FileSink;
use sghtmltopdf_core::style::{compute_styles, extract_author_stylesheet, user_agent_stylesheet};

struct Options {
    input: PathBuf,
    font: PathBuf,
    output: PathBuf,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("エラー: {message}");
            eprintln!();
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    match run(&options) {
        Ok(page_count) => {
            eprintln!(
                "{}ページのPDFを書き出しました: {}",
                page_count,
                options.output.display()
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("エラー: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("使い方: sghtmltopdf <input.html> --font <font.ttf> [-o <output.pdf>]");
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input = None;
    let mut font = None;
    let mut output = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--font" => {
                i += 1;
                let value = args.get(i).ok_or("--fontには値が必要です")?;
                font = Some(PathBuf::from(value));
            }
            "-o" | "--output" => {
                i += 1;
                let value = args.get(i).ok_or("-o/--outputには値が必要です")?;
                output = Some(PathBuf::from(value));
            }
            other if input.is_none() => input = Some(PathBuf::from(other)),
            other => return Err(format!("不明な引数です: {other}")),
        }
        i += 1;
    }

    let input = input.ok_or("入力HTMLファイルを指定してください")?;
    let font = font.ok_or("--fontでフォントファイルを指定してください")?;
    let output = output.unwrap_or_else(|| input.with_extension("pdf"));

    Ok(Options {
        input,
        font,
        output,
    })
}

fn run(options: &Options) -> Result<usize, String> {
    let html_bytes = std::fs::read(&options.input)
        .map_err(|e| format!("{}の読み込みに失敗しました: {e}", options.input.display()))?;
    let dom = html::parse(&html_bytes);

    let font =
        Font::load(&options.font).map_err(|e| format!("フォントの読み込みに失敗しました: {e}"))?;

    let ua = user_agent_stylesheet();
    let author = extract_author_stylesheet(&dom);
    let styles = compute_styles(&dom, &ua, &author);

    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &font, &settings);
    let page_count = pages.len();

    let sink = FileSink::create(&options.output)
        .map_err(|e| format!("{}の作成に失敗しました: {e}", options.output.display()))?;
    write_document(&pages, &styles, &font, &settings, sink)
        .map_err(|e| format!("PDFの書き込みに失敗しました: {e}"))?;

    Ok(page_count)
}
