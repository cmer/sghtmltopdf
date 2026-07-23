//! 文書内での画像取得結果のメモ化。
//!
//! 同じ`src`が同一文書内で何度参照されても、URL/パス分類
//! ([`classify_img_src`])から実際のフェッチ/読み込み([`ImageFetcher`])
//! までを初回の1回だけで済ませる
//! ([0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)
//! の方針)。単一スレッド前提で`Rc`/`RefCell`により共有する(このプロジェクトの
//! DOM構築・エンジン処理は一貫して単一スレッドであり、`html/parse.rs`の
//! `Sink`も同様の前提で`RefCell`を使っている)。
//!
//! 成功したバイト列だけでなく失敗(取得不能・非対応srcなど)も記録する。
//! 同じ壊れた`src`が文書中に何百回出てきても、毎回ネットワークタイムアウトを
//! 待ち直したりはしない([0014]が挙げる「ブロック対象URLを埋め込むことで
//! 処理全体を遅延させる」可用性上の懸念への対処を兼ねる)。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::ImageFetcher;

/// キャッシュ1件分の結果(成功時のバイト列、または失敗理由)。
type CachedFetch = Result<Rc<[u8]>, Rc<str>>;

/// 文書1つ分の画像取得結果キャッシュ。
///
/// 現時点(T47)ではフェッチ結果の生バイト列をキャッシュする。デコード
/// 結果やPDF `Ref`の共有(box tree構築時にRefを払い出すT51以降の話)は
/// このキャッシュの上位で扱う想定で、ここでは「同じ`src`に対して
/// フェッチ処理を繰り返さない」という最小限の役割に留める。
#[derive(Default)]
pub struct DocumentImageCache {
    entries: RefCell<HashMap<String, CachedFetch>>,
}

impl DocumentImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `raw_src`(`<img src>`属性の生の値)に対応するバイト列を返す。
    ///
    /// 初回はURL/パス分類→`fetcher`による取得までを行い、結果(成功・
    /// 失敗いずれも)をキャッシュする。2回目以降はキャッシュを`Rc`で
    /// 共有するだけで、再分類・再フェッチは行わない。
    pub fn get_or_fetch(&self, fetcher: &ImageFetcher, raw_src: &str) -> CachedFetch {
        if let Some(cached) = self.entries.borrow().get(raw_src) {
            return cached.clone();
        }

        // 分類の前に`<base href>`に対する解決を挟むため、`classify_img_src`は
        // 直接呼ばず`ImageFetcher::resolve`を経由する([0040](
        // ../../../docs/decisions/0040-base-href-design.md)決定1)。
        let result: Result<Vec<u8>, String> = fetcher
            .resolve(raw_src)
            .ok_or_else(|| format!("サポートされていないsrcです: {raw_src}"))
            .and_then(|src| fetcher.fetch(&src).map_err(|e| e.to_string()));
        let result: Result<Rc<[u8]>, Rc<str>> =
            result.map(Rc::from).map_err(|e| Rc::from(e.as_str()));

        self.entries
            .borrow_mut()
            .insert(raw_src.to_string(), result.clone());
        result
    }

    /// 現在キャッシュされている異なる`src`の件数(テスト用)。
    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-img-cache-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn caches_a_successful_local_fetch_and_shares_the_same_bytes() {
        let dir = temp_dir("success");
        std::fs::write(dir.join("logo.png"), b"logo bytes").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let first = cache.get_or_fetch(&fetcher, "logo.png").unwrap();
        let second = cache.get_or_fetch(&fetcher, "logo.png").unwrap();

        assert_eq!(&*first, b"logo bytes");
        assert!(
            Rc::ptr_eq(&first, &second),
            "repeated lookups of the same src should share the cached Rc, not re-fetch"
        );
        assert_eq!(cache.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn caches_a_failure_so_it_is_not_retried() {
        let dir = temp_dir("failure");
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let first = cache.get_or_fetch(&fetcher, "does-not-exist.png");
        let second = cache.get_or_fetch(&fetcher, "does-not-exist.png");

        assert!(first.is_err());
        match (first, second) {
            (Err(a), Err(b)) => assert!(
                Rc::ptr_eq(&a, &b),
                "the cached failure should be shared, not recomputed"
            ),
            other => panic!("expected both lookups to fail: {other:?}"),
        }
        assert_eq!(cache.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn distinct_srcs_get_distinct_cache_entries() {
        let dir = temp_dir("distinct");
        std::fs::write(dir.join("a.png"), b"a").unwrap();
        std::fs::write(dir.join("b.png"), b"b").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        assert_eq!(&*cache.get_or_fetch(&fetcher, "a.png").unwrap(), b"a");
        assert_eq!(&*cache.get_or_fetch(&fetcher, "b.png").unwrap(), b"b");
        assert_eq!(cache.len(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unsupported_src_is_cached_as_a_failure() {
        let dir = temp_dir("unsupported");
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let result = cache.get_or_fetch(&fetcher, "file:///etc/passwd");
        assert!(result.is_err());
        assert_eq!(cache.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_data_uri_does_not_need_the_fetcher_to_touch_disk_or_network() {
        let dir = temp_dir("data_uri");
        let fetcher = ImageFetcher::new(dir.clone(), false);
        let cache = DocumentImageCache::new();

        let bytes = cache
            .get_or_fetch(&fetcher, "data:image/png;base64,aGk=")
            .unwrap();
        assert_eq!(&*bytes, b"hi");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
