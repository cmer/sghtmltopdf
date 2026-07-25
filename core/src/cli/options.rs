//! CLIオプションの定義。
//!
//! [0055](../../../docs/decisions/0055-cli-design.md)決定6により、
//! **オプション定義はこのファイル1箇所だけ**に置く。HTTPサーバモード
//! (Phase 7)はクエリ文字列を引数列へ機械変換して同じパーサへ通すため、
//! CLIとサーバで解釈がずれない。
//!
//! オプション名はwkhtmltopdf互換を優先する(対応表は
//! `docs/wkhtmltopdf_option_mapping.md`)。

use std::path::PathBuf;

use clap::{ArgAction, ArgMatches, Args, Parser, Subcommand, ValueEnum};

/// 入力・出力に`-`を指定したときの意味(stdin/stdout)。
pub const STD_STREAM: &str = "-";

#[derive(Debug, Parser)]
#[command(
    name = "sghtmltopdf",
    version,
    about = "Chromium/WebKit/Geckoに依存しないHTML→PDFレンダラー",
    // 変換をサブコマンドにせず、位置引数のまま扱う([0055]決定3)。
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub convert: ConvertArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// HTTPサーバとして待ち受ける(M12 Phase 7で実装)
    Server(ServerArgs),
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    /// 待ち受けアドレス
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub listen: String,
}

/// HTML→PDF変換のオプション。
#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// 入力HTMLファイル(`-`で標準入力)
    #[arg(value_name = "INPUT.HTML", required = true)]
    pub input: Option<String>,

    /// 出力先PDF(既定は入力の拡張子を.pdfにしたもの。`-`で標準出力)
    #[arg(short, long, value_name = "OUTPUT.PDF")]
    pub output: Option<String>,

    /// 使用するフォントファイル(複数指定可)
    #[arg(long, value_name = "PATH", required = true)]
    pub font: Vec<PathBuf>,

    /// 直前の--fontに対する、TrueType Collection内のフェイス番号
    #[arg(long, value_name = "N")]
    pub font_index: Vec<u32>,

    /// `font-family: sans-serif`の実体として使うフォント
    #[arg(long, value_name = "PATH")]
    pub gothic_font: Option<PathBuf>,

    /// --gothic-fontのフェイス番号
    #[arg(long, value_name = "N", requires = "gothic_font")]
    pub gothic_font_index: Option<u32>,

    /// 相対参照の解決基準(ディレクトリかhttp(s)のURL。標準入力から読む場合に使う)
    #[arg(long, value_name = "URL|DIR")]
    pub base_url: Option<String>,

    /// <img src>/<link rel=stylesheet href>のhttp(s)フェッチを許可する
    #[arg(long, action = ArgAction::SetTrue)]
    pub allow_remote_assets: bool,

    /// ログの詳細度
    #[arg(long, value_enum, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,

    /// --log-level noneと同じ
    #[arg(short, long, action = ArgAction::SetTrue)]
    pub quiet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevel {
    None,
    Error,
    Warn,
    Info,
}

/// フォントファイルとフェイス番号の組。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontArg {
    pub path: PathBuf,
    pub index: u32,
}

impl ConvertArgs {
    /// 実効的なログ出力可否(`--quiet`は`--log-level none`と同義)。
    pub fn is_quiet(&self) -> bool {
        self.quiet || self.log_level == LogLevel::None
    }

    /// `--font`と`--font-index`を**コマンドラインでの出現順**に基づいて
    /// 組にする。
    ///
    /// `--font-index`は「直前の`--font`に対する指定」という位置依存の意味を
    /// 持つ(手書きパーサ時代からの互換)。clapは値をオプションごとにまとめて
    /// しまうため、`ArgMatches::indices_of`で元の位置を取り直して対応付ける。
    pub fn font_specs(&self, matches: &ArgMatches) -> Result<Vec<FontArg>, String> {
        let font_positions: Vec<usize> = matches
            .indices_of("font")
            .map(|it| it.collect())
            .unwrap_or_default();
        let index_positions: Vec<usize> = matches
            .indices_of("font_index")
            .map(|it| it.collect())
            .unwrap_or_default();

        let mut specs: Vec<FontArg> = self
            .font
            .iter()
            .map(|path| FontArg {
                path: path.clone(),
                index: 0,
            })
            .collect();

        for (nth, position) in index_positions.iter().enumerate() {
            // その`--font-index`より手前にある`--font`のうち最後のもの。
            let target = font_positions.iter().rposition(|p| p < position);
            match target {
                Some(i) => specs[i].index = self.font_index[nth],
                None => {
                    return Err("--font-indexは直前の--fontに対して指定してください".to_string())
                }
            }
        }

        Ok(specs)
    }

    /// `--gothic-font`とそのフェイス番号。
    pub fn gothic_font_spec(&self) -> Option<FontArg> {
        self.gothic_font.as_ref().map(|path| FontArg {
            path: path.clone(),
            index: self.gothic_font_index.unwrap_or(0),
        })
    }

    /// 入力が標準入力か。
    pub fn reads_stdin(&self) -> bool {
        self.input.as_deref() == Some(STD_STREAM)
    }

    /// 出力先。`-o`省略時は入力の拡張子を`.pdf`に置き換える。
    /// 標準出力の場合は`None`を返す。
    pub fn output_path(&self) -> Result<Option<PathBuf>, String> {
        match self.output.as_deref() {
            Some(STD_STREAM) => Ok(None),
            Some(path) => Ok(Some(PathBuf::from(path))),
            None => {
                if self.reads_stdin() {
                    return Err(
                        "標準入力から読む場合は-o/--outputで出力先を指定してください(標準出力は`-o -`)"
                            .to_string(),
                    );
                }
                let input = PathBuf::from(self.input.as_deref().unwrap_or_default());
                Ok(Some(input.with_extension("pdf")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> (Cli, ArgMatches) {
        let matches = Cli::command().get_matches_from(args);
        let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches).unwrap();
        (cli, matches)
    }

    #[test]
    fn font_index_applies_to_the_preceding_font() {
        let (cli, matches) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--font",
            "b.ttc",
            "--font-index",
            "2",
            "--font",
            "c.ttf",
        ]);
        let specs = cli.convert.font_specs(&matches).unwrap();
        assert_eq!(
            specs,
            vec![
                FontArg {
                    path: PathBuf::from("a.ttf"),
                    index: 0
                },
                FontArg {
                    path: PathBuf::from("b.ttc"),
                    index: 2
                },
                FontArg {
                    path: PathBuf::from("c.ttf"),
                    index: 0
                },
            ]
        );
    }

    #[test]
    fn font_index_before_any_font_is_an_error() {
        let (cli, matches) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font-index",
            "1",
            "--font",
            "a.ttf",
        ]);
        assert!(cli.convert.font_specs(&matches).is_err());
    }

    #[test]
    fn output_defaults_to_the_input_with_pdf_extension() {
        let (cli, _) = parse(&["sghtmltopdf", "docs/in.html", "--font", "a.ttf"]);
        assert_eq!(
            cli.convert.output_path().unwrap(),
            Some(PathBuf::from("docs/in.pdf"))
        );
    }

    #[test]
    fn dash_selects_std_streams() {
        let (cli, _) = parse(&["sghtmltopdf", "-", "--font", "a.ttf", "-o", "-"]);
        assert!(cli.convert.reads_stdin());
        assert_eq!(cli.convert.output_path().unwrap(), None);
    }

    #[test]
    fn stdin_input_requires_an_explicit_output() {
        let (cli, _) = parse(&["sghtmltopdf", "-", "--font", "a.ttf"]);
        assert!(cli.convert.output_path().is_err());
    }

    #[test]
    fn quiet_is_equivalent_to_log_level_none() {
        let (cli, _) = parse(&["sghtmltopdf", "in.html", "--font", "a.ttf", "-q"]);
        assert!(cli.convert.is_quiet());
        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--log-level",
            "none",
        ]);
        assert!(cli.convert.is_quiet());
        let (cli, _) = parse(&["sghtmltopdf", "in.html", "--font", "a.ttf"]);
        assert!(!cli.convert.is_quiet());
    }

    #[test]
    fn server_subcommand_does_not_require_convert_args() {
        let (cli, _) = parse(&["sghtmltopdf", "server", "--listen", "0.0.0.0:9000"]);
        match cli.command {
            Some(Command::Server(ref args)) => assert_eq!(args.listen, "0.0.0.0:9000"),
            _ => panic!("server subcommand should be parsed"),
        }
    }

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
