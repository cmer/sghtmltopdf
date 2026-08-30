//! Ruby拡張のエントリポイント。
//!
//! この層は薄く保つ。オプションの引数列(argv)への組み立てはRuby側が
//! 行い、ここは受け取ったargvをCLI・HTTPサーバと同じパーサへ通して
//! レンダリングするだけ。

mod callback_sink;
mod errors;
mod gvl;

use std::io::Cursor;
use std::path::PathBuf;

use magnus::rb_sys::AsRawValue;
use magnus::{block::Proc, function, prelude::*, Error, RString, Ruby};
use sghtmltopdf_core::cli::convert;
use sghtmltopdf_core::render_stack;
use sghtmltopdf_core::sink::{FileSink, MemorySink};

use callback_sink::{pump_to_block, BlockSlot, PendingUnwind, ValueSlot};

/// HTMLを変換してPDFのバイト列を返す。
fn render(html: RString, argv: Vec<String>) -> Result<RString, Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    // GVLを解放する前にRust側へコピーする。解放中はRubyのオブジェクトに
    // 触れないため、`RString`のままでは持ち込めない。
    let html = unsafe { html.as_slice() }.to_vec();
    errors::catch_panic(&ruby, move || render_inner(html, argv))
}

fn render_inner(html: Vec<u8>, argv: Vec<String>) -> Result<RString, Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    let (args, fonts) = cli::parse_convert_argv(&argv).map_err(|e| errors::to_ruby(&ruby, e))?;

    // GVLを解放したうえで、さらにレンダリング専用のスタックを確保した
    // スレッドへ移す。Rubyのスレッドのマシンスタックは既定1MiBしかなく、
    // レイアウト・描画の再帰に耐えられないため(`callback_sink`のモジュール
    // doc参照)。この経路はRubyへコールバックしないので、そのまま移せる。
    let pdf = gvl::without_gvl(move || {
        render_stack::with_render_stack(move || {
            convert::render_to_memory(&args, &fonts, Cursor::new(html), MemorySink::new())
        })
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
    errors::catch_panic(&ruby, move || render_to_file_inner(html, argv, path))
}

fn render_to_file_inner(html: Vec<u8>, argv: Vec<String>, path: String) -> Result<(), Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    let (args, fonts) = cli::parse_convert_argv(&argv).map_err(|e| errors::to_ruby(&ruby, e))?;

    let path = PathBuf::from(path);
    let sink = FileSink::create(&path).map_err(|e| {
        errors::to_ruby(
            &ruby,
            cli::CliError::Input(format!("{}の作成に失敗しました: {e}", path.display())),
        )
    })?;

    gvl::without_gvl(move || {
        render_stack::with_render_stack(move || convert::render(&args, &fonts, Cursor::new(html), sink))
    })
    .map_err(|e| errors::to_ruby(&ruby, e))?;
    Ok(())
}

/// HTMLを変換し、確定したPDFのバイト列を`chunk_size`ごとに`block`へ渡す。
///
/// レンダリングの間はGVLを解放し、ブロックを呼ぶ瞬間だけ取り戻す。ブロックが
/// 例外を投げた場合は、その例外をそのまま呼び出し元へ伝える(エンジン側は
/// 通常のエラーパスで巻き戻る)。
fn render_each(
    html: RString,
    argv: Vec<String>,
    block: Proc,
    chunk_size: usize,
) -> Result<(), Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    let html = unsafe { html.as_slice() }.to_vec();
    errors::catch_panic(&ruby, move || {
        render_each_inner(html, argv, block, chunk_size)
    })
}

fn render_each_inner(
    html: Vec<u8>,
    argv: Vec<String>,
    block: Proc,
    chunk_size: usize,
) -> Result<(), Error> {
    let ruby = Ruby::get().expect("GVLを保持したまま呼ばれるはず");
    let (args, fonts) = cli::parse_convert_argv(&argv).map_err(|e| errors::to_ruby(&ruby, e))?;

    // ブロックはGVL解放区間をまたいで生きる必要があるため、GCへ登録する
    // (解放後にスタックへ積んだ値は保守的GCの走査対象外)。
    let block = ValueSlot::new(block.as_raw());
    let mut pending = PendingUnwind::default();

    let result = {
        let slot = BlockSlot::new(&block);
        let pending = &mut pending;
        gvl::without_gvl(move || {
            // レンダリングは専用スタックのスレッドで走り、確定したチャンクだけが
            // ここへ戻ってくる。ブロックの呼び出し(=GVLの再取得)は、GVLを
            // 手放したこのスレッドで行う必要があるため`pump_to_block`に任せる。
            pump_to_block(slot, pending, chunk_size, move |sink| {
                convert::render(&args, &fonts, Cursor::new(html), sink)
            })
        })
    };
    drop(block);

    // ブロック由来の中断は、エンジンが返すエラーより優先して伝える
    // (`Sink::Error`が`io::Error`固定のため、理由はこちらに載っている)。
    // `break`等の脱出は`into_error`の中で`rb_jump_tag`し戻らないので、
    // Rust側の値はここで落としきってから呼ぶ。
    if pending.is_pending() {
        drop(result);
        return Err(pending
            .into_error()
            .expect("is_pendingがtrueなら中断が入っている"));
    }
    result.map_err(|e| errors::to_ruby(&ruby, e))
}

/// coreへリンクできていることの確認用(疎通確認)。
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
    native.define_singleton_method("render_each", function!(render_each, 4))?;
    native.define_singleton_method("core_version", function!(core_version, 0))?;
    native.define_singleton_method("default_page_size", function!(default_page_size, 0))?;
    native.define_singleton_method("sleep_without_gvl", function!(sleep_without_gvl, 1))?;
    Ok(())
}
