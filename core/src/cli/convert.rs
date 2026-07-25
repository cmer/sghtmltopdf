//! 変換サブコマンド(サブコマンド省略時の既定)の実行。

use std::io::{self, Read};
use std::path::PathBuf;

use clap::ArgMatches;

use crate::engine::{Engine, EngineError, EngineOptions, FontSpec as EngineFontSpec};
use crate::sink::{FileSink, Sink, StdoutSink};

use super::options::ConvertArgs;
use super::CliError;

/// 出力先(ファイル/標準出力)を1つの型にまとめる。
/// [`Engine`]は`S: Sink`のジェネリックなので、分岐をここで吸収する。
enum OutputSink {
    File(FileSink),
    Stdout(StdoutSink),
}

impl Sink for OutputSink {
    type Output = ();
    type Error = io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        match self {
            Self::File(sink) => sink.write(bytes),
            Self::Stdout(sink) => sink.write(bytes),
        }
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        match self {
            Self::File(sink) => sink.finish(),
            Self::Stdout(sink) => sink.finish(),
        }
    }
}

pub fn run(args: &ConvertArgs, matches: &ArgMatches) -> Result<(), CliError> {
    let fonts = args.font_specs(matches).map_err(CliError::Usage)?;
    let output_path = args.output_path().map_err(CliError::Usage)?;

    let html_bytes = read_input(args)?;
    // 入力をUTF-8へ揃える(BOM > --encoding > <meta charset> > UTF-8)。
    let html =
        crate::html::decode_html(&html_bytes, args.encoding.as_deref()).map_err(CliError::Usage)?;
    let (base_dir, base_href) = resolve_base(args)?;

    // CLIのページ設定は「初期値」であり、著者CSSの`@page`宣言があれば
    // プロパティ単位でそちらが優先される([0055]決定2)。合成は
    // `engine::apply_page_rule_settings_override`が行う。
    let settings = args.page_settings().map_err(CliError::Usage)?;
    args.validate_scaling().map_err(CliError::Usage)?;
    let content_options = args.content_options().map_err(CliError::Input)?;

    let engine_options = EngineOptions {
        mode: args.mode(),
        settings,
        fonts: fonts
            .iter()
            .map(|spec| EngineFontSpec {
                path: spec.path.clone(),
                index: spec.index,
            })
            .collect(),
        generic_fonts: args
            .generic_font_specs()
            .into_iter()
            .map(|(family, spec)| {
                (
                    family,
                    EngineFontSpec {
                        path: spec.path,
                        index: spec.index,
                    },
                )
            })
            .collect(),
        base_dir,
        base_href,
        allow_remote_assets: args.allow_remote_assets,
        output: args.pdf_output_options(),
        content: content_options,
        local_access: args.local_access(),
    };

    let sink = match output_path.as_ref() {
        Some(path) => OutputSink::File(FileSink::create(path).map_err(|e| {
            CliError::Input(format!("{}の作成に失敗しました: {e}", path.display()))
        })?),
        None => OutputSink::Stdout(StdoutSink::new()),
    };

    let mut engine = Engine::new(engine_options, sink);
    engine.feed(html.as_bytes()).map_err(engine_error)?;
    engine.finish().map_err(engine_error)?;

    if !args.is_quiet() {
        match output_path.as_ref() {
            Some(path) => eprintln!("PDFを書き出しました: {}", path.display()),
            None => eprintln!("PDFを標準出力へ書き出しました"),
        }
    }
    Ok(())
}

/// `EngineError`をexit code([0055]決定4)へ対応付ける。
/// 書き込み失敗・フォント読み込み失敗はリソースエラー(2)、エンジン自身の
/// 制約違反はレンダリングエラー(3)。
fn engine_error(e: EngineError<io::Error>) -> CliError {
    match e {
        EngineError::Io(e) => CliError::Input(format!("PDFの書き込みに失敗しました: {e}")),
        EngineError::Font(msg) => CliError::Input(msg),
        EngineError::UnsupportedInStreamingMode(msg) => CliError::Render(msg.to_string()),
        EngineError::MediaLoad(msg) => {
            CliError::Input(format!("リソースの取得に失敗しました: {msg}"))
        }
    }
}

fn read_input(args: &ConvertArgs) -> Result<Vec<u8>, CliError> {
    if args.reads_stdin() {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| CliError::Input(format!("標準入力の読み込みに失敗しました: {e}")))?;
        return Ok(buf);
    }

    let path = PathBuf::from(args.input.as_deref().unwrap_or_default());
    std::fs::read(&path)
        .map_err(|e| CliError::Input(format!("{}の読み込みに失敗しました: {e}", path.display())))
}

/// 相対参照の解決基準を決める。
///
/// * `--base-url`がhttp(s)のURLなら`<base href>`の既定値として渡す
///   (HTML内に`<base href>`があればそちらが優先される)
/// * `--base-url`がディレクトリならローカル解決の基準ディレクトリにする
/// * 未指定なら入力HTMLのあるディレクトリ(標準入力の場合はカレント)
fn resolve_base(args: &ConvertArgs) -> Result<(Option<PathBuf>, Option<String>), CliError> {
    let input_dir = if args.reads_stdin() {
        None
    } else {
        PathBuf::from(args.input.as_deref().unwrap_or_default())
            .parent()
            .map(|p| p.to_path_buf())
    };

    let Some(base_url) = args.base_url.as_deref() else {
        return Ok((input_dir, None));
    };

    let lower = base_url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok((input_dir, Some(base_url.to_string())));
    }

    let dir = PathBuf::from(base_url);
    if !dir.is_dir() {
        return Err(CliError::Input(format!(
            "--base-urlにはディレクトリかhttp(s)のURLを指定してください: {base_url}"
        )));
    }
    Ok((Some(dir), None))
}
