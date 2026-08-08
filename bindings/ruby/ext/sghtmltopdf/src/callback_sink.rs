//! 確定したPDFのバイト列を、Rubyのブロックへチャンクごとに渡す仕組み。
//!
//! # スレッドの分け方
//!
//! レンダリングはDOMの深さぶん再帰する(スタイル計算・レイアウト・描画)。
//! Rubyのスレッドのマシンスタックは既定1MiB(`RubyVM::DEFAULT_PARAMS`の
//! `thread_machine_stack_size`)しかなく、Pumaのワーカースレッド上でそのまま
//! 走らせると深さ200弱でスタックを溢れさせる。しかもGVLを解放した状態で
//! ガードページに触れるため、プロセスが落ちるのではなくスレッドが固まる。
//!
//! そこでレンダリングは[`sghtmltopdf_core::cli::STACK_SIZE`]のスタックを
//! 明示的に確保した専用スレッドで走らせ、確定したチャンクはチャネル越しに
//! 元のスレッドへ渡す。Rubyへ触れるのは元のスレッドだけに限る。
//!
//! ```text
//! 元のスレッド(Rubyが作った / GVL解放中)     レンダリングスレッド(16MiB)
//!   recv(chunk)  <---------- chunk ----------  Sink::write
//!   with_gvl { block.call(chunk) }
//!   send(ack)    ------------ ack ---------->  (次のチャンクへ)
//! ```
//!
//! この向きでないと成立しない: [`crate::gvl::with_gvl`]の
//! `rb_thread_call_with_gvl`は「そのスレッドが`rb_thread_call_without_gvl`で
//! GVLを手放している」ことが前提で、Rubyの知らないスレッドから呼ぶことは
//! できない。だからレンダリングスレッドはRubyに一切触れない。

use std::io;
use std::sync::mpsc::{Receiver, SyncSender};

use magnus::rb_sys::{AsRawValue, FromRawValue};
use magnus::{block::Proc, Error, ExceptionClass, RString, Ruby, Value};
use rb_sys::VALUE;
use sghtmltopdf_core::sink::Sink;

use crate::gvl;

/// ブロックが中断したときに`Sink::write`が返すエラー。
///
/// `convert::render`が`Sink<Output = (), Error = io::Error>`を要求するため、
/// Ruby由来の情報をエラーの型に載せられない。本当の理由は
/// [`PendingUnwind`]へ置き、こちらは「巻き戻すための合図」として使う。
fn interrupted() -> io::Error {
    io::Error::other("Rubyのブロックが中断しました")
}

/// `rb_gc_register_address`でGCから守った`VALUE`の置き場。
///
/// Rubyの保守的GCはマシンスタックを走査してVALUEを見つけるが、
/// GVLを解放した時点のスタック位置までしか走査しない
/// (解放時にマシンコンテキストが保存されるため)。`without_gvl`の内側で
/// スタックに積んだ値はその先にあるので走査されない。解放区間をまたいで
/// 生かしたいVALUEは、必ずここへ登録する。
///
/// 登録アドレスは`Box`で固定する。GCのコンパクションでオブジェクトが移動
/// しても、登録したアドレスの中身は更新されるため、古い参照を掴まない。
pub struct ValueSlot {
    slot: Box<VALUE>,
}

impl ValueSlot {
    pub fn new(value: VALUE) -> Self {
        let mut slot = Box::new(value);
        unsafe { rb_sys::rb_gc_register_address(&mut *slot) };
        Self { slot }
    }

    /// GC登録済みスロットのアドレス。
    pub fn addr(&self) -> *mut VALUE {
        &*self.slot as *const VALUE as *mut VALUE
    }

    /// 現在の`VALUE`。GVLを保持している間だけ呼ぶこと。
    pub fn get(&self) -> VALUE {
        *self.slot
    }
}

impl Drop for ValueSlot {
    fn drop(&mut self) {
        unsafe { rb_sys::rb_gc_unregister_address(&mut *self.slot) };
    }
}

/// GVL解放区間へ運ぶための、[`ValueSlot`]のアドレス。
#[derive(Clone, Copy)]
pub struct BlockSlot(*mut VALUE);

// SAFETY: 解放区間ではアドレスを数値として持ち回るだけで、`VALUE`として
// 読むのは`with_gvl`の内側(＝GVLを保持している間)に限る。指す先は
// `ValueSlot`がGCに登録済みで、`without_gvl`が返るまで生きている。
unsafe impl Send for BlockSlot {}

impl BlockSlot {
    pub fn new(slot: &ValueSlot) -> Self {
        Self(slot.addr())
    }

    /// GVLを保持している前提で`Proc`へ戻す。
    fn proc(self) -> Option<Proc> {
        let value = unsafe { Value::from_raw(*self.0) };
        Proc::from_value(value)
    }
}

/// ブロックが投げた例外・脱出を、GVL解放区間の外へ運ぶための受け皿。
#[derive(Default)]
pub struct PendingUnwind {
    unwind: Option<Unwind>,
}

enum Unwind {
    /// Rubyの例外オブジェクト。
    Exception(ValueSlot),
    /// Rust側(magnus)が組み立てたエラー。クラスとメッセージを別々に運ぶ。
    Raise { class: ValueSlot, message: String },
    /// `break`・`return`・`throw`など。値は`rb_jump_tag`へ渡すタグ。
    Jump(i32),
}

impl PendingUnwind {
    /// ブロックが中断していれば`true`。
    pub fn is_pending(&self) -> bool {
        self.unwind.is_some()
    }

    /// magnusの`Error`を、解放区間をまたげる形に変換して保存する。
    /// GVLを保持している間に呼ぶこと(GCへの登録を行うため)。
    fn store(&mut self, error: Error) {
        use magnus::error::ErrorType;

        let unwind = match error.error_type() {
            ErrorType::Jump(tag) => Unwind::Jump(*tag as i32),
            ErrorType::Error(class, message) => Unwind::Raise {
                class: ValueSlot::new(class.as_raw()),
                message: message.to_string(),
            },
            ErrorType::Exception(exception) => {
                Unwind::Exception(ValueSlot::new(exception.as_raw()))
            }
        };
        self.unwind = Some(unwind);
    }

    /// 保存した中断をRubyへ返す。GVLを保持している間に呼ぶこと。
    ///
    /// `break`などの脱出は`rb_jump_tag`で忠実に伝播させる。この関数は
    /// そこから戻らないため、Rust側の後始末が済んでから呼ぶこと
    /// (`ValueSlot`のGC登録解除もこの関数の中で済ませてある)。
    pub fn into_error(self) -> Option<Error> {
        match self.unwind? {
            Unwind::Exception(slot) => {
                let value = unsafe { Value::from_raw(slot.get()) };
                // 登録を外すのは`Error`を組み立てたあと。ここから先は
                // 呼び出し元がGVLを保持したままRubyへ戻るので、
                // 保守的GCの走査範囲に入る。
                let error = magnus::Exception::from_value(value).map(Error::from);
                drop(slot);
                Some(error.unwrap_or_else(|| {
                    Error::new(
                        Ruby::get()
                            .expect("GVLを保持したまま呼ばれるはず")
                            .exception_runtime_error(),
                        "ブロックが投げた例外を復元できませんでした",
                    )
                }))
            }
            Unwind::Raise { class, message } => {
                let value = unsafe { Value::from_raw(class.get()) };
                let error = ExceptionClass::from_value(value).map(|c| Error::new(c, message));
                drop(class);
                Some(error.unwrap_or_else(|| {
                    Error::new(
                        Ruby::get()
                            .expect("GVLを保持したまま呼ばれるはず")
                            .exception_runtime_error(),
                        "ブロックの中断を復元できませんでした",
                    )
                }))
            }
            // `rb_jump_tag`は戻らない(`-> !`)。`self`の他のフィールドは
            // ここまでで全部落ちている。
            Unwind::Jump(tag) => unsafe { rb_sys::rb_jump_tag(tag) },
        }
    }
}

/// 確定したバイト列を`chunk_size`ごとにチャネルへ流すSink。
///
/// レンダリングスレッド側で使う。Rubyには一切触れないので、`Send`であり
/// GVLの制約とも無縁。1チャンク送るごとに受け取り側の応答を待つ
/// (rendezvous)ことで、ブロックの処理より先に走ってメモリを溜め込まない。
pub struct ChannelSink {
    chunks: SyncSender<Vec<u8>>,
    ack: Receiver<bool>,
    buf: Vec<u8>,
    chunk_size: usize,
}

impl ChannelSink {
    fn new(chunks: SyncSender<Vec<u8>>, ack: Receiver<bool>, chunk_size: usize) -> Self {
        Self {
            chunks,
            ack,
            buf: Vec::new(),
            // 0だと1バイトごとにGVLを取り直すことになるため下限を設ける。
            chunk_size: chunk_size.max(1),
        }
    }

    /// 1チャンク渡して、ブロックが受け取り終えるまで待つ。
    ///
    /// 送れない(受け取り側が降りた)場合と、ブロックが中断を返した場合は
    /// どちらも[`interrupted`]で巻き戻す。中断の本当の理由は受け取り側の
    /// [`PendingUnwind`]に入っている。
    fn hand_off(&mut self, chunk: Vec<u8>) -> Result<(), io::Error> {
        if self.chunks.send(chunk).is_err() {
            return Err(interrupted());
        }
        match self.ack.recv() {
            Ok(true) => Ok(()),
            _ => Err(interrupted()),
        }
    }
}

impl Sink for ChannelSink {
    type Output = ();
    type Error = io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        self.buf.extend_from_slice(bytes);
        while self.buf.len() >= self.chunk_size {
            let chunk: Vec<u8> = self.buf.drain(..self.chunk_size).collect();
            self.hand_off(chunk)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), io::Error> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let rest = std::mem::take(&mut self.buf);
        self.hand_off(rest)
    }
}

/// 1チャンクをRubyのブロックへ渡す。中断したら`false`を返す。
///
/// GVLを取り戻すのはこの中だけ。ブロックの呼び出しはmagnusの`Proc::call`が
/// 内部で`rb_protect`しているので、例外が出てもlongjmpがRustのフレームを
/// 飛び越えない。
///
/// エラーの保存もこの区間の中で済ませる。`with_gvl`からRubyのオブジェクト
/// (例外)を持ち出すと、GVLを手放した瞬間にGCのスコープから外れてしまう
/// ため(`gvl::with_gvl`のドキュメント)。持ち出すのは真偽値だけ。
fn call_block(block: BlockSlot, pending: &mut PendingUnwind, bytes: Vec<u8>) -> bool {
    gvl::with_gvl(move || {
        let ruby = Ruby::get().expect("with_gvlの内側なのでGVLを持っている");
        let result = match block.proc() {
            Some(proc) => {
                let chunk: RString = ruby.str_from_slice(&bytes);
                proc.call::<_, Value>((chunk,)).map(|_| ())
            }
            None => Err(Error::new(
                ruby.exception_runtime_error(),
                "ブロックが失われました",
            )),
        };
        match result {
            Ok(()) => true,
            Err(error) => {
                // GCへの登録もGVLを持っているこの場で行う。
                pending.store(error);
                false
            }
        }
    })
}

/// `render`をレンダリング専用スレッドで走らせ、出てきたチャンクをこのスレッド
/// からRubyのブロックへ渡し続ける。
///
/// GVLを解放している区間(`without_gvl`の内側)から、その解放したスレッド上で
/// 呼ぶこと。モジュールdocの図のうち左側がこの関数にあたる。
pub fn pump_to_block<F>(
    block: BlockSlot,
    pending: &mut PendingUnwind,
    chunk_size: usize,
    render: F,
) -> Result<(), sghtmltopdf_core::cli::CliError>
where
    F: FnOnce(ChannelSink) -> Result<(), sghtmltopdf_core::cli::CliError> + Send + 'static,
{
    use sghtmltopdf_core::cli::{CliError, STACK_SIZE};

    // どちらも容量0のrendezvous。レンダリング側は1チャンクごとに
    // ブロックの完了を待つ。
    let (chunk_tx, chunk_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(0);
    let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel::<bool>(0);

    let worker = std::thread::Builder::new()
        .name("sghtmltopdf-render".to_string())
        .stack_size(STACK_SIZE)
        .spawn(move || render(ChannelSink::new(chunk_tx, ack_rx, chunk_size)))
        .map_err(|e| CliError::Input(format!("レンダリングスレッドを作れません: {e}")))?;

    while let Ok(chunk) = chunk_rx.recv() {
        let ok = call_block(block, pending, chunk);
        // 応答を返せない(レンダリング側が既に降りた)場合も抜ける。
        if ack_tx.send(ok).is_err() || !ok {
            break;
        }
    }
    // 中断で抜けた場合、レンダリング側が次のsendでエラーになって巻き戻れる
    // よう、受け口を先に落とす。
    drop(chunk_rx);
    drop(ack_tx);

    worker
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
