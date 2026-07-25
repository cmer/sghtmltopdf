//! GVL(Global VM Lock)の解放。
//!
//! レンダリングの間はGVLを解放し、Pumaの他スレッドを止めないようにする
//! ([0062](../../../../../docs/decisions/0062-ruby-binding.md)決定6)。
//!
//! magnus 0.8にこのラッパは無い(`magnus`の`lib.rs`が挙げる未実装API一覧に
//! `rb_thread_call_without_gvl`が入っている)ため、rb-sysのバインディングを
//! 直接呼ぶ。

use std::ffi::c_void;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// GVLを解放して`func`を実行する。
///
/// # なぜ`Send`境界が要るか
///
/// magnusは**「GVLを解放するAPIは存在しない」前提**でGVL状態をスレッド
/// ローカルにキャッシュしており(`magnus::api`の *assumed not to change
/// because there's currently no api to unlock*)、ここで解放しても
/// `Ruby::get()`は`Ok`を返してしまう。返ったハンドルでRubyに触ればUBになる。
///
/// `Send`境界を課すと、magnusの値(`NonNull<RBasic>`)も`Ruby`ハンドル
/// (`*mut ()`)も`!Send`なのでキャプチャがコンパイルエラーになる。
/// クロージャの中で改めて`Ruby::get()`を呼ぶことまでは型では防げないが、
/// 解放区間で呼ぶのは`sghtmltopdf_core`の関数だけであり、コアはRubyを
/// 一切知らないため到達しない。
///
/// # 割り込み
///
/// UBF(unblock function)は`None`＝割り込み不可。`Kernel#trap`やCtrl-Cでの
/// 中断は初期スコープ外とする(決定6)。
#[allow(dead_code)] // Phase 2(T338)で使う
pub fn without_gvl<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    struct State<F, R> {
        func: Option<F>,
        result: Option<std::thread::Result<R>>,
    }

    unsafe extern "C" fn call<F, R>(arg: *mut c_void) -> *mut c_void
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        let state = unsafe { &mut *(arg as *mut State<F, R>) };
        let func = state.func.take().expect("コールバックが2度呼ばれました");
        // パニックがFFI境界を越えるとプロセスがabortするため、ここで捕まえて
        // GVLを取り戻してからRust側でresumeする。
        state.result = Some(catch_unwind(AssertUnwindSafe(func)));
        std::ptr::null_mut()
    }

    let mut state = State::<F, R> {
        func: Some(func),
        result: None,
    };
    unsafe {
        rb_sys::rb_thread_call_without_gvl(
            Some(call::<F, R>),
            &mut state as *mut _ as *mut c_void,
            None,
            std::ptr::null_mut(),
        );
    }
    match state.result.expect("コールバックが実行されませんでした") {
        Ok(value) => value,
        Err(panic) => resume_unwind(panic),
    }
}
