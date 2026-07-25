//! Ruby拡張のエントリポイント。
//!
//! 設計は[0062](../../../../../docs/decisions/0062-ruby-binding.md)。
//! この層は薄く保ち、オプションのargv組み立てはRuby側で行う(決定2)。

mod gvl;

use magnus::{function, prelude::*, Error, Ruby};

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
    let native = module.define_module("Native")?;
    native.define_singleton_method("core_version", function!(core_version, 0))?;
    native.define_singleton_method("default_page_size", function!(default_page_size, 0))?;
    native.define_singleton_method("sleep_without_gvl", function!(sleep_without_gvl, 1))?;
    Ok(())
}
