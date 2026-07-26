//! Ruby拡張のエントリポイント。
//!
//! 設計は[0062](../../../../../docs/decisions/0062-ruby-binding.md)。
//! **この層は薄く保つ**。オプションの引数列(argv)への組み立てはRuby側が行い
//! (決定2)、ここは受け取ったargvをCLI・HTTPサーバと同じパーサへ通して
//! レンダリングするだけ。

mod errors;
mod gvl;

use std::io::Cursor;
use std::path::PathBuf;

use magnus::{function, prelude::*, Error, RString, Ruby};
use sghtmltopdf_core::cli::{self, convert};
use sghtmltopdf_core::sink::{FileSink, MemorySink};

/// HTMLを変換してPDFのバイト列を返す。
fn render(html: RString, argv: Vec<String>) -> Result<RString, Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    // GVLを解放する前にRust側へコピーする。解放中はRubyのオブジェクトに
    // 触れないため(決定6)、`RString`のままでは持ち込めない。
    let html = unsafe { html.as_slice() }.to_vec();
    let (args, fonts) = cli::parse_convert_argv(&argv).map_err(|e| errors::to_ruby(&ruby, e))?;

    let pdf = gvl::without_gvl(move || {
        convert::render_to_memory(&args, &fonts, Cursor::new(html), MemorySink::new())
    })
    .map_err(|e| errors::to_ruby(&ruby, e))?;

    Ok(ruby.str_from_slice(&pdf))
}

/// HTMLを変換して`path`へ書き出す。
///
/// 出力先は[`FileSink`]が決めるので、argvの`--output`は使われない
/// (一時ファイルへ書いて成功時だけrenameするため、途中で失敗しても
/// 壊れたPDFが残らない)。
fn render_to_file(html: RString, argv: Vec<String>, path: String) -> Result<(), Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    let html = unsafe { html.as_slice() }.to_vec();
    let (args, fonts) = cli::parse_convert_argv(&argv).map_err(|e| errors::to_ruby(&ruby, e))?;

    let path = PathBuf::from(path);
    let sink = FileSink::create(&path).map_err(|e| {
        errors::to_ruby(
            &ruby,
            cli::CliError::Input(format!("{}の作成に失敗しました: {e}", path.display())),
        )
    })?;

    gvl::without_gvl(move || convert::render(&args, &fonts, Cursor::new(html), sink))
        .map_err(|e| errors::to_ruby(&ruby, e))?;
    Ok(())
}

/// coreへリンクできていることの確認用(Phase 0の疎通確認)。
fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// coreのシンボルを実際に1つ呼んでリンクを確かめる。
fn default_page_size() -> String {
    let settings = sghtmltopdf_core::layout::PageSettings::default();
    format!("{}x{}", settings.size.width, settings.size.height)
}

/// GVLを解放して実行できることの確認用。解放中も他のRubyスレッドが
/// 進めることをRuby側のテストで検証する。
fn sleep_without_gvl(ms: u64) {
    gvl::without_gvl(|| std::thread::sleep(std::time::Duration::from_millis(ms)));
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Sghtmltopdf")?;
    errors::define(ruby, module)?;

    let native = module.define_module("Native")?;
    native.define_singleton_method("render", function!(render, 2))?;
    native.define_singleton_method("render_to_file", function!(render_to_file, 3))?;
    native.define_singleton_method("core_version", function!(core_version, 0))?;
    native.define_singleton_method("default_page_size", function!(default_page_size, 0))?;
    native.define_singleton_method("sleep_without_gvl", function!(sleep_without_gvl, 1))?;
    Ok(())
}
