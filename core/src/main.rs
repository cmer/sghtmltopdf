//! sghtmltopdf CLIのエントリポイント。
//!
//! 実装は[`sghtmltopdf_core::cli`]にあり、ここはそれを呼ぶだけの薄い層
//! ([0055](../../docs/decisions/0055-cli-design.md)決定1)。

use std::process::ExitCode;

fn main() -> ExitCode {
    sghtmltopdf_core::cli::run()
}
