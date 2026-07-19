//! sg-core: HTML → PDF レンダリングエンジンのコア実装。
//!
//! Ruby/Railsに一切依存しない独立したクレート。
//! マイルストーン1: 静的HTML一括変換(ストリーミングなし)で
//! 基本的なブロック/インラインレイアウト + PDF出力ができる状態を目指す。
//! マイルストーン3で、これらを1つのAPIとして統合する[`engine::Engine`]を
//! 追加した。

pub mod engine;
pub mod fonts;
pub mod html;
pub mod layout;
pub mod pdf;
pub mod sink;
pub mod style;
