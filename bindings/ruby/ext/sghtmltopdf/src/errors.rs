//! Rubyの例外クラスと、コアの[`CliError`]からの対応付け。

use std::panic::AssertUnwindSafe;

use magnus::{prelude::*, ExceptionClass, RModule, Ruby};
use sghtmltopdf_core::cli::CliError;

pub fn define(ruby: &Ruby, module: RModule) -> Result<(), magnus::Error> {
    let base = module.define_error("Error", ruby.exception_standard_error())?;
    module.define_error("UsageError", base)?;
    module.define_error("InputError", base)?;
    module.define_error("RenderError", base)?;
    module.define_error("TimeoutError", base)?;
    module.define_error("InternalError", base)?;
    Ok(())
}

/// Rustのパニックを`Sghtmltopdf::InternalError`へ変換して`f`を実行する。
///
/// # なぜ自前で捕まえるか
///
/// magnusもメソッド呼び出しをパニックから守っており、プロセスがabortする
/// ことはない。ただしmagnusが変換する先はRubyの`fatal`で、これは
/// `rescue Exception`でも捕まえられずプロセスが終了する。Webアプリの中で
/// 1リクエストぶんのバグのためにワーカーごと落ちるのは困るので、
/// magnusへ渡る前にここで`StandardError`の子孫へ変換する。
///
/// パニックはコアの不具合を意味するので、握りつぶさずメッセージを残す。
pub fn catch_panic<F, R>(ruby: &Ruby, f: F) -> Result<R, magnus::Error>
where
    F: FnOnce() -> Result<R, magnus::Error>,
{
    // AssertUnwindSafe: パニックで巻き戻った後に触るのはRuby側の例外生成だけで、
    // Rust側の壊れかけた状態を読み直すことはない。
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(magnus::Error::new(
            class(ruby, "InternalError"),
            format!("内部エラー(パニック): {}", panic_message(&payload)),
        )),
    }
}

/// パニックのペイロードから人が読めるメッセージを取り出す。
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "詳細不明".to_string()
    }
}

/// コアのエラーを、対応するRubyの例外へ変換する。
///
/// メッセージはコアが返す文言をそのまま使う（CLIと同じ文言になる）。
pub fn to_ruby(ruby: &Ruby, error: CliError) -> magnus::Error {
    let (class_name, message) = match error {
        CliError::Usage(message) => ("UsageError", message),
        CliError::Input(message) => ("InputError", message),
        CliError::Render(message) => ("RenderError", message),
        CliError::Timeout(message) => ("TimeoutError", message),
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
