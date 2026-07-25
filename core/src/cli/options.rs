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

use crate::layout::{PageSettings, PageSize};
use crate::pdf::{DocumentMetadata, PdfOutputOptions};

use super::units::parse_length_px;

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

    /// 用紙サイズ
    #[arg(short = 's', long, value_enum, ignore_case = true, value_name = "SIZE")]
    pub page_size: Option<PageSizeName>,

    /// 用紙の幅(--page-sizeより優先。単位はmm/cm/in/pt/px、省略時はmm)
    #[arg(long, value_name = "LENGTH")]
    pub page_width: Option<String>,

    /// 用紙の高さ(--page-sizeより優先)
    #[arg(long, value_name = "LENGTH")]
    pub page_height: Option<String>,

    /// 用紙の向き(Landscapeは最終的な幅と高さを入れ替える)
    #[arg(short = 'O', long, value_enum, ignore_case = true)]
    pub orientation: Option<Orientation>,

    /// 上マージン(既定1in)
    #[arg(short = 'T', long, value_name = "LENGTH")]
    pub margin_top: Option<String>,

    /// 下マージン(既定1in)
    #[arg(short = 'B', long, value_name = "LENGTH")]
    pub margin_bottom: Option<String>,

    /// 左マージン(既定1in)
    #[arg(short = 'L', long, value_name = "LENGTH")]
    pub margin_left: Option<String>,

    /// 右マージン(既定1in)
    #[arg(short = 'R', long, value_name = "LENGTH")]
    pub margin_right: Option<String>,

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

    /// PDFのタイトル(未指定ならHTMLの<title>を使う)
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,

    /// PDFの著者(Info辞書の/Author)
    #[arg(long, value_name = "TEXT")]
    pub author: Option<String>,

    /// PDFの主題(Info辞書の/Subject)
    #[arg(long, value_name = "TEXT")]
    pub subject: Option<String>,

    /// PDFのキーワード(Info辞書の/Keywords)
    #[arg(long, value_name = "TEXT")]
    pub keywords: Option<String>,

    /// CSS pxを何dpiとして解釈するか(既定96。72にすると1px=1pt)
    #[arg(short = 'd', long, value_name = "DPI", default_value_t = 96.0)]
    pub dpi: f32,

    /// 拡大率(既定1.0)
    #[arg(long, value_name = "FACTOR", default_value_t = 1.0)]
    pub zoom: f32,

    /// 塗り・線の色をグレースケールにする
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    pub grayscale: bool,

    /// PDFオブジェクトのFlate圧縮を行わない(画像データは対象外)
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_pdf_compression: bool,

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

/// `--page-size`で選べる用紙。CSSの`@page { size: ... }`が受け付ける
/// キーワードと同じ集合にしてある。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PageSizeName {
    #[value(name = "A3")]
    A3,
    #[value(name = "A4")]
    A4,
    #[value(name = "A5")]
    A5,
    #[value(name = "Letter")]
    Letter,
    #[value(name = "Legal")]
    Legal,
}

impl PageSizeName {
    fn to_page_size(self) -> PageSize {
        match self {
            Self::A3 => PageSize::A3,
            Self::A4 => PageSize::A4,
            Self::A5 => PageSize::A5,
            Self::Letter => PageSize::LETTER,
            Self::Legal => PageSize::LEGAL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Orientation {
    #[value(name = "Portrait")]
    Portrait,
    #[value(name = "Landscape")]
    Landscape,
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

    /// ページサイズ・マージンのCLI指定を[`PageSettings`]へまとめる。
    ///
    /// ここで返すのは**初期値**であり、著者CSSに`@page`の宣言があれば
    /// プロパティ単位でそちらが優先される
    /// ([0055](../../../docs/decisions/0055-cli-design.md)決定2。合成は
    /// `engine::apply_page_rule_settings_override`が行う)。
    ///
    /// `--page-width`/`--page-height`は`--page-size`より優先し、
    /// `--orientation Landscape`は**最後に**幅と高さを入れ替える。
    pub fn page_settings(&self) -> Result<PageSettings, String> {
        let defaults = PageSettings::default();

        let mut size = self
            .page_size
            .map(PageSizeName::to_page_size)
            .unwrap_or(defaults.size);
        if let Some(value) = self.page_width.as_deref() {
            size.width = parse_length_px(value)?;
        }
        if let Some(value) = self.page_height.as_deref() {
            size.height = parse_length_px(value)?;
        }
        if self.orientation == Some(Orientation::Landscape) {
            size = size.landscape();
        }
        if size.width <= 0.0 || size.height <= 0.0 {
            return Err("用紙の幅と高さには正の値を指定してください".to_string());
        }

        let mut margin = defaults.margin;
        for (value, edge) in [
            (self.margin_top.as_deref(), &mut margin.top),
            (self.margin_bottom.as_deref(), &mut margin.bottom),
            (self.margin_left.as_deref(), &mut margin.left),
            (self.margin_right.as_deref(), &mut margin.right),
        ] {
            if let Some(value) = value {
                *edge = parse_length_px(value)?;
            }
        }

        let settings = PageSettings { size, margin };
        if settings.content_width() <= 0.0 {
            return Err("左右マージンの合計が用紙の幅以上です".to_string());
        }
        if settings.content_height() <= 0.0 {
            return Err("上下マージンの合計が用紙の高さ以上です".to_string());
        }
        Ok(settings)
    }

    /// PDF書き出しオプションへまとめる([0057](
    /// ../../../docs/decisions/0057-pdf-output-options-design.md))。
    ///
    /// `--title`が未指定の場合の`<title>`フォールバックはエンジン側で行う。
    pub fn pdf_output_options(&self) -> PdfOutputOptions {
        PdfOutputOptions {
            metadata: DocumentMetadata {
                title: self.title.clone(),
                author: self.author.clone(),
                subject: self.subject.clone(),
                keywords: self.keywords.clone(),
            },
            compress: !self.no_pdf_compression,
            scale: PdfOutputOptions::scale_from_dpi_and_zoom(self.dpi, self.zoom),
            grayscale: self.grayscale,
        }
    }

    /// `--dpi`/`--zoom`の値の妥当性(正の有限値であること)。
    pub fn validate_scaling(&self) -> Result<(), String> {
        if !(self.dpi.is_finite() && self.dpi > 0.0) {
            return Err(format!("--dpiには正の値を指定してください: {}", self.dpi));
        }
        if !(self.zoom.is_finite() && self.zoom > 0.0) {
            return Err(format!("--zoomには正の値を指定してください: {}", self.zoom));
        }
        Ok(())
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
    fn page_size_name_is_case_insensitive_and_maps_to_the_layout_constants() {
        let (cli, _) = parse(&["sghtmltopdf", "in.html", "--font", "a.ttf", "-s", "a5"]);
        let settings = cli.convert.page_settings().unwrap();
        assert_eq!(settings.size, PageSize::A5);
    }

    #[test]
    fn explicit_width_and_height_win_over_page_size() {
        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--page-size",
            "A4",
            "--page-width",
            "400px",
            "--page-height",
            "500px",
        ]);
        let settings = cli.convert.page_settings().unwrap();
        assert_eq!(settings.size.width, 400.0);
        assert_eq!(settings.size.height, 500.0);
    }

    #[test]
    fn landscape_swaps_width_and_height_last() {
        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--page-width",
            "400px",
            "--page-height",
            "500px",
            "-O",
            "Landscape",
        ]);
        let settings = cli.convert.page_settings().unwrap();
        assert_eq!(settings.size.width, 500.0);
        assert_eq!(settings.size.height, 400.0);
    }

    #[test]
    fn margins_default_to_one_inch_and_are_overridden_individually() {
        let (cli, _) = parse(&["sghtmltopdf", "in.html", "--font", "a.ttf"]);
        let settings = cli.convert.page_settings().unwrap();
        assert_eq!(settings.margin.top, 96.0);
        assert_eq!(settings.margin.left, 96.0);

        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "-T",
            "25.4mm",
            "--margin-left",
            "0",
        ]);
        let settings = cli.convert.page_settings().unwrap();
        assert!((settings.margin.top - 96.0).abs() < 0.01);
        assert_eq!(settings.margin.left, 0.0);
        // 指定しなかった辺は既定のまま。
        assert_eq!(settings.margin.right, 96.0);
    }

    #[test]
    fn margins_larger_than_the_page_are_rejected() {
        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--page-width",
            "100px",
            "--margin-left",
            "60px",
            "--margin-right",
            "60px",
        ]);
        assert!(cli.convert.page_settings().is_err());
    }

    #[test]
    fn a_bad_length_is_reported_as_an_error() {
        let (cli, _) = parse(&[
            "sghtmltopdf",
            "in.html",
            "--font",
            "a.ttf",
            "--margin-top",
            "10em",
        ]);
        assert!(cli.convert.page_settings().is_err());
    }

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
