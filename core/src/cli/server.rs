//! `server`サブコマンド(HTTPサーバモード)。
//!
//! 中身はM12 Phase 7(T314〜T320)で実装する。ここではサブコマンドの枠だけを
//! 用意し、指定された場合は未実装であることを明示して終了する
//! ([0055](../../../docs/decisions/0055-cli-design.md)決定5と同じ考え方で、
//! 黙って何もしないことは避ける)。

use super::options::ServerArgs;
use super::CliError;

pub fn run(args: &ServerArgs) -> Result<(), CliError> {
    Err(CliError::Usage(format!(
        "serverサブコマンドはまだ実装されていません(M12 Phase 7で対応予定)。\n  \
         指定された待ち受けアドレス: {}",
        args.listen
    )))
}
