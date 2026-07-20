//! `<img>`要素まわりの処理(マイルストーン5: 画像埋め込み)。
//!
//! [0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)の
//! 方針により、フェッチ・デコード・PDF埋め込みはbox tree構築時(T52)に
//! 遅延して行う。このモジュールはその前段として、DOMから`<img>`要素の
//! 属性(URL解決前の生の値)を読み取る部分(T44)を担う。

mod attrs;
mod cache;
mod fetch;
mod resolve;

pub use attrs::{read_img_attrs, ImgAttrs};
pub use cache::DocumentImageCache;
pub use fetch::{FetchError, ImageFetcher};
pub use resolve::{classify_img_src, ImgSrc};
