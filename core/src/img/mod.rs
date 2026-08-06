//! `<img>`要素まわりの処理(マイルストーン5: 画像埋め込み)。
//!
//! フェッチ・デコード・PDF埋め込みはbox tree構築時(T52)に遅延して行う。この
//! モジュールはその前段として、DOMから`<img>`要素の属性(URL解決前の生の値)を
//! 読み取る部分(T44)を担う。

mod attrs;
mod cache;
mod fetch;
mod resolve;

pub use attrs::{read_img_attrs, ImgAttrs};
pub use cache::DocumentImageCache;
pub use fetch::{FetchError, ImageFetcher};
pub use resolve::{
    classify_img_src, resolve_against_base_href, resolve_local_asset_path, ImgSrc,
    ResolvedAssetPath,
};
