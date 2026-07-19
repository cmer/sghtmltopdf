//! sghtmltopdf CLI: HTMLファイルを一括変換してPDFを出力する。
//!
//! M1では静的HTML一括変換(ストリーミングなし)のみ対応。フォントは
//! `--font`での明示指定(必須、複数指定可)に加えて、HTML内`<style>`の
//! `@font-face { src: url(...); }`(HTMLファイル自身のディレクトリ基準の相対解決)、
//! およびOS標準フォントディレクトリのシステムフォント探索(`fontdb`。CSS汎用
//! family名は対象外で、具体的なfont-family名のみ)にも対応する。複数フォントが
//! 対象になった場合、CSSの`font-family`と各フォントのグリフカバレッジに基づいて
//! フォールバック選択される([`sghtmltopdf_core::fonts::FontCollection`])。

use std::path::PathBuf;
use std::process::ExitCode;

use sghtmltopdf_core::fonts::{
    load_font_faces, load_missing_system_fonts, Font, FontCollection, SystemFonts,
};
use sghtmltopdf_core::html;
use sghtmltopdf_core::layout::{paginate_document, PageSettings};
use sghtmltopdf_core::pdf::write_document;
use sghtmltopdf_core::sink::FileSink;
use sghtmltopdf_core::style::{compute_styles, extract_author_stylesheet, user_agent_stylesheet};

struct FontSpec {
    path: PathBuf,
    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイルのフェイス番号。
    index: u32,
}

struct Options {
    input: PathBuf,
    fonts: Vec<FontSpec>,
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
    eprintln!(
        "使い方: sghtmltopdf <input.html> --font <font.ttf> [--font <font2.ttf> [--font-index N]]... [-o <output.pdf>]"
    );
    eprintln!(
        "  --font-indexは直前の--fontに対して、TrueType Collection(.ttc)内のフェイス番号を指定する(既定は0)"
    );
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input = None;
    let mut fonts: Vec<FontSpec> = Vec::new();
    let mut output = None;

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
    })
}

fn run(options: &Options) -> Result<usize, String> {
    let html_bytes = std::fs::read(&options.input)
        .map_err(|e| format!("{}の読み込みに失敗しました: {e}", options.input.display()))?;
    let dom = html::parse(&html_bytes);

    let mut loaded_fonts = Vec::with_capacity(options.fonts.len());
    for spec in &options.fonts {
        let font = Font::load_indexed(&spec.path, spec.index)
            .map_err(|e| format!("フォントの読み込みに失敗しました: {e}"))?;
        loaded_fonts.push(font);
    }
    let mut fonts = FontCollection::new(loaded_fonts);

    let ua = user_agent_stylesheet();
    let author = extract_author_stylesheet(&dom);
    let styles = compute_styles(&dom, &ua, &author);

    // `@font-face`のsrc: url(...)は、HTMLファイル自身のディレクトリを基準に
    // 相対パス解決する(外部CSSファイルという概念が無く、HTMLの<style>のみが
    // CSSの入力元のため)。
    let base_dir = options.input.parent().unwrap_or(std::path::Path::new("."));
    for loaded in load_font_faces(&author.font_faces, base_dir) {
        fonts.push_font_face(
            loaded.family,
            Some(loaded.weight),
            Some(loaded.style),
            loaded.font,
        );
    }

    // `--font`/`@font-face`のどちらでも解決できなかった具体的なfont-family名を
    // OS標準のフォントディレクトリから探す。
    let system_fonts = SystemFonts::scan();
    load_missing_system_fonts(&mut fonts, &styles, &system_fonts);

    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let page_count = pages.len();

    let sink = FileSink::create(&options.output)
        .map_err(|e| format!("{}の作成に失敗しました: {e}", options.output.display()))?;
    write_document(&pages, &styles, &fonts, &settings, sink)
        .map_err(|e| format!("PDFの書き込みに失敗しました: {e}"))?;

    Ok(page_count)
}
