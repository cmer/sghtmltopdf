//! CLI(`sghtmltopdf`バイナリ)の実装。
//!
//! 設計は[0055](../../../docs/decisions/0055-cli-design.md)。
//! `main.rs`は[`run`]を呼ぶだけの薄いエントリで、オプション定義は
//! [`options`]の1箇所に集約する(決定6。HTTPサーバモードも同じ定義を使う)。

pub mod convert;
pub mod header_footer;
pub mod options;
pub mod server;
pub mod toc;
pub mod units;
pub mod unsupported;

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches};

use options::{Cli, Command};

/// CLIのエラー。バリアントがそのままexit code([0055]決定4)に対応する。
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

/// CLIのエントリポイント。
pub fn run() -> ExitCode {
    // wkhtmltopdfにあって対応していないオプションは、clapの「unknown
    // argument」ではなく**理由と代替手段を示して**終了する([0055]決定5)。
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(message) = unsupported::check_arguments(&args) {
        eprintln!("エラー: {message}");
        return ExitCode::from(1);
    }

    // clapは既定で引数エラーにexit code 2を使うが、[0055]決定4では
    // 使用法エラーを1に割り当てているため、自前でExitCodeへ変換する。
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

    let result = match cli.command {
        Some(Command::Server(ref args)) => server::run(args),
        None => convert::run(&cli.convert, &matches),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}
