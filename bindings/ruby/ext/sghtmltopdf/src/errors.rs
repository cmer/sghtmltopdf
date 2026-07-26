//! Rubyの例外クラスと、コアの[`CliError`]からの対応付け。
//!
//! ([0062](../../../../../docs/decisions/0062-ruby-binding.md)決定9)

use magnus::{prelude::*, ExceptionClass, RModule, Ruby};
use sghtmltopdf_core::cli::CliError;

/// `Sghtmltopdf::Error`とその子クラスを定義する。
pub fn define(ruby: &Ruby, module: RModule) -> Result<(), magnus::Error> {
    let base = module.define_error("Error", ruby.exception_standard_error())?;
    module.define_error("UsageError", base)?;
    module.define_error("InputError", base)?;
    module.define_error("RenderError", base)?;
    Ok(())
}

/// コアのエラーを、対応するRubyの例外へ変換する。
///
/// メッセージはコアが返す文言をそのまま使う（CLIと同じ文言になる）。
pub fn to_ruby(ruby: &Ruby, error: CliError) -> magnus::Error {
    let (class_name, message) = match error {
        CliError::Usage(message) => ("UsageError", message),
        CliError::Input(message) => ("InputError", message),
        CliError::Render(message) => ("RenderError", message),
    };
    magnus::Error::new(class(ruby, class_name), message)
}

/// `Sghtmltopdf::<name>`の例外クラスを引く。
///
/// 定義は`.so`のロード時に済んでいる（[`define`]）。万一引けなかった場合は
/// エラーを握りつぶさずに`RuntimeError`として上げる。
pub fn class(ruby: &Ruby, name: &str) -> ExceptionClass {
    ruby.class_object()
        .const_get::<_, RModule>("Sghtmltopdf")
        .and_then(|module| module.const_get::<_, ExceptionClass>(name))
        .unwrap_or_else(|_| ruby.exception_runtime_error())
}
