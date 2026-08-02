//! GVL(Global VM Lock)の解放。
//!
//! レンダリングの間はGVLを解放し、Pumaの他スレッドを止めないようにする。
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
/// magnusは「GVLを解放するAPIは存在しない」前提でGVL状態をスレッド
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
/// 中断は初期スコープ外とする。
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

/// GVLを取り戻して`func`を実行する。[`without_gvl`]の内側からだけ呼ぶ。
///
/// `without_gvl`と違い`Send`境界は課さない。ここはGVLを保持している＝Rubyに
/// 触ってよい区間だから。
///
/// # 呼び出し側が守ること(libruby側の制約)
///
/// * `func`からRubyのオブジェクトを返さない。返すとGVLを再び手放した
///   あとGCのスコープから外れ、マークされない。値を持ち帰るときは
///   `rb_gc_register_address`で登録したスロットへ入れること
///   (`callback_sink::ValueSlot`)
/// * `func`から例外を投げさせない。longjmpがこの関数を飛び越えると
///   未定義動作になる。Rubyの呼び出しは必ず`rb_protect`相当で包む
///   (magnusの`Proc::call`は内部で`protect`しているのでそのまま使える)
pub fn with_gvl<F, R>(func: F) -> R
where
    F: FnOnce() -> R,
{
    struct State<F, R> {
        func: Option<F>,
        result: Option<std::thread::Result<R>>,
    }

    unsafe extern "C" fn call<F, R>(arg: *mut c_void) -> *mut c_void
    where
        F: FnOnce() -> R,
    {
        let state = unsafe { &mut *(arg as *mut State<F, R>) };
        let func = state.func.take().expect("コールバックが2度呼ばれました");
        // パニックがlibrubyのフレームを越えるとプロセスがabortする。
        state.result = Some(catch_unwind(AssertUnwindSafe(func)));
        std::ptr::null_mut()
    }

    let mut state = State::<F, R> {
        func: Some(func),
        result: None,
    };
    unsafe {
        rb_sys::rb_thread_call_with_gvl(Some(call::<F, R>), &mut state as *mut _ as *mut c_void);
    }
    match state.result.expect("コールバックが実行されませんでした") {
        Ok(value) => value,
        Err(panic) => resume_unwind(panic),
    }
}
