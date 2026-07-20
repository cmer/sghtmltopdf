//! sghtmltopdf CLI: HTMLファイルを一括変換してPDFを出力する。
//!
//! フォントは`--font`での明示指定(必須、複数指定可)に加えて、HTML内`<style>`の
//! `@font-face { src: url(...); }`(HTMLファイル自身のディレクトリ基準の相対解決)/
//! `src: local(...)`(システムフォントのフルネーム/PostScript名解決)、
//! およびOS標準フォントディレクトリのシステムフォント探索(`fontdb`。CSS汎用
//! family名は対象外で、具体的なfont-family名のみ)にも対応する。複数フォントが
//! 対象になった場合、CSSの`font-family`と各フォントのグリフカバレッジに基づいて
//! フォールバック選択される([`sghtmltopdf_core::fonts::FontCollection`])。
//!
//! マイルストーン3以降、内部実装は[`sghtmltopdf_core::engine::Engine`]
//! (`Mode::Batch`)を経由する。CLIは常に入力ファイルを一括で`feed`するため、
//! 挙動自体はM1時点の一括変換と変わらない。

use std::path::PathBuf;
use std::process::ExitCode;

use sghtmltopdf_core::engine::{Engine, EngineOptions, FontSpec as EngineFontSpec, Mode};
use sghtmltopdf_core::layout::PageSettings;
use sghtmltopdf_core::sink::FileSink;

struct FontSpec {
    path: PathBuf,
    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイルのフェイス番号。
    index: u32,
}

struct Options {
    input: PathBuf,
    fonts: Vec<FontSpec>,
    output: PathBuf,
    allow_remote_assets: bool,
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
        Ok(()) => {
            eprintln!("PDFを書き出しました: {}", options.output.display());
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("エラー: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "使い方: sghtmltopdf <input.html> --font <font.ttf> [--font <font2.ttf> [--font-index N]]... [-o <output.pdf>] [--allow-remote-assets]"
    );
    eprintln!(
        "  --font-indexは直前の--fontに対して、TrueType Collection(.ttc)内のフェイス番号を指定する(既定は0)"
    );
    eprintln!(
        "  --allow-remote-assetsは<img src>/<link rel=stylesheet href>のhttp(s)絶対URLフェッチを許可する(既定は無効。ローカル相対パス/data:URIは常に許可)"
    );
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input = None;
    let mut fonts: Vec<FontSpec> = Vec::new();
    let mut output = None;
    let mut allow_remote_assets = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--font" => {
                i += 1;
                let value = args.get(i).ok_or("--fontには値が必要です")?;
                fonts.push(FontSpec {
                    path: PathBuf::from(value),
                    index: 0,
                });
            }
            "--font-index" => {
                i += 1;
                let value = args.get(i).ok_or("--font-indexには値が必要です")?;
                let index: u32 = value
                    .parse()
                    .map_err(|_| format!("--font-indexは数値で指定してください: {value}"))?;
                let last = fonts
                    .last_mut()
                    .ok_or("--font-indexは直前の--fontに対して指定してください")?;
                last.index = index;
            }
            "-o" | "--output" => {
                i += 1;
                let value = args.get(i).ok_or("-o/--outputには値が必要です")?;
                output = Some(PathBuf::from(value));
            }
            "--allow-remote-assets" => {
                allow_remote_assets = true;
            }
            other if input.is_none() => input = Some(PathBuf::from(other)),
            other => return Err(format!("不明な引数です: {other}")),
        }
        i += 1;
    }

    let input = input.ok_or("入力HTMLファイルを指定してください")?;
    if fonts.is_empty() {
        return Err("--fontでフォントファイルを指定してください(複数指定可)".to_string());
    }
    let output = output.unwrap_or_else(|| input.with_extension("pdf"));

    Ok(Options {
        input,
        fonts,
        output,
        allow_remote_assets,
    })
}

fn run(options: &Options) -> Result<(), String> {
    let html_bytes = std::fs::read(&options.input)
        .map_err(|e| format!("{}の読み込みに失敗しました: {e}", options.input.display()))?;

    let engine_fonts = options
        .fonts
        .iter()
        .map(|spec| EngineFontSpec {
            path: spec.path.clone(),
            index: spec.index,
        })
        .collect();

    // `@font-face`のsrc: url()・<img src>・<link rel=stylesheet href>いずれの
    // ローカル相対パスも、HTMLファイル自身のディレクトリを基準に解決する。
    let base_dir = options.input.parent().map(|p| p.to_path_buf());

    let engine_options = EngineOptions {
        mode: Mode::Batch,
        settings: PageSettings::default(),
        fonts: engine_fonts,
        base_dir,
        allow_remote_assets: options.allow_remote_assets,
    };

    let sink = FileSink::create(&options.output)
        .map_err(|e| format!("{}の作成に失敗しました: {e}", options.output.display()))?;

    let mut engine = Engine::new(engine_options, sink);
    engine
        .feed(&html_bytes)
        .map_err(|e| format!("HTMLの処理に失敗しました: {e}"))?;
    engine
        .finish()
        .map_err(|e| format!("PDFの書き込みに失敗しました: {e}"))?;

    Ok(())
}
