//! CLI(`sghtmltopdf`バイナリ)の実装。
//!
//! `main.rs`は[`run`]を呼ぶだけの薄いエントリで、オプション定義は
//! [`options`]の1箇所に集約する(HTTPサーバモードも同じ定義を使う)。

pub mod convert;
pub mod header_footer;
pub mod options;
/// HTTPサーバモード。`server` feature(既定ON)でのみ有効。
#[cfg(feature = "server")]
pub mod server;
pub mod toc;
pub mod units;
pub mod unsupported;

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches};

use options::Cli;
#[cfg(feature = "server")]
use options::Command;

/// CLIのエラー。バリアントがそのままexit codeに対応する。
#[derive(Debug)]
pub enum CliError {
    /// 使用法エラー(不明なオプション、値の形式不正、非対応オプション) = 1
    Usage(String),
    /// 入力・リソースエラー(ファイルが無い、フォントが読めない、書き込み失敗) = 2
    Input(String),
    /// レンダリングエラー(エンジンの制約違反など) = 3
    Render(String),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 1,
            Self::Input(_) => 2,
            Self::Render(_) => 3,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Usage(m) | Self::Input(m) | Self::Render(m) => m,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CliError {}

/// 引数列を解析して[`options::ConvertArgs`]とフォント指定を返す。
///
/// CLI以外の入口（Ruby binding）が同じオプション解釈を使うための関数。
/// 呼び出し側は`argv[0]`にプログラム名を置くこと（clapの慣習に合わせる）。
///
/// `--font`と`--font-index`の対応付けには[`clap::ArgMatches`]の出現位置が
/// 必要なため、ここで解決して[`options::FontArg`]の列にして返す
/// （呼び出し側がclapに依存せずに済む）。
pub fn parse_convert_argv(
    argv: &[String],
) -> Result<(options::ConvertArgs, Vec<options::FontArg>), CliError> {
    // 非対応オプションは、clapの「unknown argument」
    // ではなく理由を示す。CLIの`run`と同じ扱いにする。
    if let Some(message) = unsupported::check_arguments(&argv[1..]) {
        return Err(CliError::Usage(message));
    }

    let matches = Cli::command()
        .try_get_matches_from(argv)
        .map_err(|e| CliError::Usage(e.to_string()))?;
    let cli = Cli::from_arg_matches(&matches).map_err(|e| CliError::Usage(e.to_string()))?;
    let fonts = cli.convert.font_specs(&matches).map_err(CliError::Usage)?;
    Ok((cli.convert, fonts))
}

/// CLIのエントリポイント。
pub fn run() -> ExitCode {
    // wkhtmltopdfにあって対応していないオプションは、clapの「unknown
    // argument」ではなく理由と代替手段を示して終了する。
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(message) = unsupported::check_arguments(&args) {
        eprintln!("エラー: {message}");
        return ExitCode::from(1);
    }

    // clapは既定で引数エラーにexit code 2を使うが、このCLIは使用法エラーを
    // 1に割り当てているため、自前でExitCodeへ変換する。
    let matches = match Cli::command().try_get_matches() {
        Ok(matches) => matches,
        Err(e) => {
            let _ = e.print();
            // --help/--versionは正常系(use_stderr()==false)。
            return if e.use_stderr() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            };
        }
    };

    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(1);
        }
    };

    #[cfg(feature = "server")]
    let result = match cli.command {
        Some(Command::Server(ref args)) => server::run(args),
        None => convert::run(&cli.convert, &matches),
    };
    #[cfg(not(feature = "server"))]
    let result = convert::run(&cli.convert, &matches);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        let mut argv = vec!["sghtmltopdf".to_string(), "-".to_string()];
        argv.extend(["--output".to_string(), "-".to_string()]);
        argv.extend(args.iter().map(|s| s.to_string()));
        argv
    }

    #[test]
    fn parse_convert_argv_binds_each_font_index_to_the_preceding_font() {
        // Ruby bindingは`--font-index`を対応する`--font`の直後に置く。その
        // 並びが期待どおり解決されること。
        let (_, fonts) = parse_convert_argv(&argv(&[
            "--font",
            "a.ttf",
            "--font",
            "b.ttc",
            "--font-index",
            "2",
        ]))
        .expect("parse should succeed");

        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].index, 0);
        assert_eq!(fonts[1].index, 2);
    }

    #[test]
    fn parse_convert_argv_rejects_unsupported_options_with_a_reason() {
        let error = parse_convert_argv(&argv(&["--enable-javascript"]))
            .expect_err("unsupported option should be rejected");

        match error {
            CliError::Usage(message) => assert!(message.contains("対応していません")),
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    #[test]
    fn parse_convert_argv_rejects_unknown_options() {
        assert!(matches!(
            parse_convert_argv(&argv(&["--no-such-option"])),
            Err(CliError::Usage(_))
        ));
    }
}
