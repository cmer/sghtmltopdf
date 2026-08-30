//! レンダリング用スタックの確保。
//!
//! スタイル計算・ボックスツリー構築・レイアウト・PDF描画はDOMの深さぶん
//! 再帰するため、呼び出し元のスレッドのスタックが小さいと深い文書で落ちる。
//! CLI・HTTPサーバ・Rubyバインディング・エンジンのテストが共通で使う。

/// レンダリングを走らせるスレッドに確保するスタックサイズ。
///
/// 必要量は[`crate::html::MAX_ELEMENT_DEPTH`]で決まる。上限256段は
/// デバッグビルド換算で約2.8MiBなので、5倍以上の余裕を取ってこの値にしている。
///
/// 既定任せにしないのは、スレッドの既定スタックが実行環境で大きく変わるため
/// (Rustの生成スレッドは2MiB、`ulimit -s`次第でmainはもっと小さくなりうる、
/// Rubyのスレッドは1MiB程度)。ここで固定しておけば、上限の判定が
/// 「実際に耐えられる深さ」と食い違わない。
pub const STACK_SIZE: usize = 16 * 1024 * 1024;

/// `f`を[`STACK_SIZE`]のスタックを持つスレッド上で実行し、結果を返す。
///
/// `f`がパニックした場合は、そのパニックを呼び出し元スレッドへ伝播させる
/// (スレッドを挟んだことで挙動が変わらないようにする)。
pub fn with_render_stack<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, f)
            .expect("レンダリング用スレッドを作れませんでした")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}
