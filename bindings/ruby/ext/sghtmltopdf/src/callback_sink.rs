//! 確定したPDFのバイト列を、Rubyのブロックへチャンクごとに渡す[`Sink`]。
//!
//! 設計は[0063](../../../../../docs/decisions/0063-ffi-chunk-callback.md)。
//!
//! レンダリングは[`crate::gvl::without_gvl`]の中で走るので、ここはGVLを
//! **解放している区間**から呼ばれる。ブロックを呼ぶ瞬間だけ
//! [`crate::gvl::with_gvl`]でGVLを取り戻す。

use std::io;

use magnus::rb_sys::{AsRawValue, FromRawValue};
use magnus::{block::Proc, Error, ExceptionClass, RString, Ruby, Value};
use rb_sys::VALUE;
use sghtmltopdf_core::sink::Sink;

use crate::gvl;

/// ブロックが中断したときに`Sink::write`が返すエラー。
///
/// `convert::render`が`Sink<Output = (), Error = io::Error>`を要求するため、
/// Ruby由来の情報をエラーの型に載せられない。**本当の理由は
/// [`PendingUnwind`]へ置き**、こちらは「巻き戻すための合図」として使う。
fn interrupted() -> io::Error {
    io::Error::other("Rubyのブロックが中断しました")
}

/// `rb_gc_register_address`でGCから守った`VALUE`の置き場。
///
/// # なぜ必要か
///
/// Rubyの保守的GCはマシンスタックを走査してVALUEを見つけるが、
/// **GVLを解放した時点のスタック位置までしか走査しない**
/// (解放時にマシンコンテキストが保存されるため)。`without_gvl`の内側で
/// スタックに積んだ値はその先にあるので**走査されない**。解放区間をまたいで
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

    /// 現在の`VALUE`。**GVLを保持している間だけ**呼ぶこと。
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

// SAFETY: 解放区間ではアドレスを**数値として持ち回るだけ**で、`VALUE`として
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
    /// **GVLを保持している間**に呼ぶこと(GCへの登録を行うため)。
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

    /// 保存した中断をRubyへ返す。**GVLを保持している間**に呼ぶこと。
    ///
    /// `break`などの脱出は`rb_jump_tag`で忠実に伝播させる。この関数は
    /// そこから戻らないため、**Rust側の後始末が済んでから**呼ぶこと
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

/// 書き出されたバイト列を、`chunk_size`ごとにRubyのブロックへ渡すSink。
pub struct CallbackSink<'a> {
    block: BlockSlot,
    pending: &'a mut PendingUnwind,
    buf: Vec<u8>,
    chunk_size: usize,
}

impl<'a> CallbackSink<'a> {
    pub fn new(block: BlockSlot, pending: &'a mut PendingUnwind, chunk_size: usize) -> Self {
        Self {
            block,
            pending,
            buf: Vec::new(),
            // 0だと1バイトごとにGVLを取り直すことになるため下限を設ける。
            chunk_size: chunk_size.max(1),
        }
    }

    /// 溜まっているバイト列をブロックへ渡す。
    ///
    /// GVLを取り戻すのはこの中だけ。ブロックの呼び出しはmagnusの
    /// `Proc::call`が内部で`rb_protect`しているので、例外が出ても
    /// **longjmpがRustのフレームを飛び越えない**([0063])。
    ///
    /// エラーの保存も**この区間の中で**済ませる。`with_gvl`から
    /// Rubyのオブジェクト(例外)を持ち出すと、GVLを手放した瞬間に
    /// GCのスコープから外れてしまうため(`gvl::with_gvl`のドキュメント)。
    /// 持ち出すのは真偽値だけ。
    fn call_block(&mut self, bytes: Vec<u8>) -> Result<(), io::Error> {
        let block = self.block;
        let pending: &mut PendingUnwind = self.pending;

        let called = gvl::with_gvl(move || {
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
        });

        if called {
            Ok(())
        } else {
            Err(interrupted())
        }
    }
}

impl Sink for CallbackSink<'_> {
    type Output = ();
    type Error = io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        // 一度中断したら、以降はブロックを呼ばずに巻き戻す。
        if self.pending.is_pending() {
            return Err(interrupted());
        }
        self.buf.extend_from_slice(bytes);
        while self.buf.len() >= self.chunk_size {
            let chunk: Vec<u8> = self.buf.drain(..self.chunk_size).collect();
            self.call_block(chunk)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), io::Error> {
        if self.pending.is_pending() {
            return Err(interrupted());
        }
        if self.buf.is_empty() {
            return Ok(());
        }
        let rest = std::mem::take(&mut self.buf);
        self.call_block(rest)
    }
}
