//! HTMLパースとDOM構築(html5ever)。

mod dom;
mod parse;

mod encoding;

pub use dom::{
    collect_anchor_targets, find_base_href, find_document_title, is_stylesheet_link, Children, Dom,
    Node, NodeData, NodeId,
};

/// 受け付けるDOMの最大深さ。これを超える入力はエラーで拒否する。
///
/// 1段あたりのスタック消費は実測で最適化ビルド約4.6KiB・デバッグビルド約11KiB
/// だった(2MiBスタックでの限界がそれぞれ深さ約450・約195)。この上限は
/// デバッグビルド換算で約2.8MiBに相当するため、レンダリングを行うスレッドには
/// [`STACK_SIZE`](crate::cli::STACK_SIZE)程度のスタックが要る。CLI・HTTPサーバ・
/// Ruby拡張はいずれも自前でスタックを確保したスレッド上で実行する。
pub const MAX_ELEMENT_DEPTH: u32 = 256;

/// 同時に保持できるノード数の上限。これを超える入力はエラーで拒否する。
///
/// DOMだけでなく、算出スタイル・ボックスツリー・レイアウト結果・ページが
/// ノード数に比例して積み上がる。実測(最適化ビルド)では1ノードあたり
/// 472B(テーブル)〜1210B(インライン要素の羅列)で、形状によらずノード数に
/// ほぼ線形だった。50万ノードなら最悪でも約600MiBに収まる。
///
/// テキスト量に比例するメモリはこの上限では抑えられない(ノード3個でも
/// 10MiBのテキストなら1.7GiB使う)。そちらはHTTPサーバの
/// `--max-body-size`が担当する。
pub const MAX_NODES: usize = 500_000;
pub use encoding::{decode_html, StreamingDecoder};
pub use parse::{parse, StreamingParser};
