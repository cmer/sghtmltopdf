//! 文書内での画像取得結果のメモ化。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::ImageFetcher;

/// キャッシュ1件分の結果(成功時のバイト列、または失敗理由)。
type CachedFetch = Result<Rc<[u8]>, Rc<str>>;

/// 文書1つ分の画像取得結果キャッシュ。
#[derive(Default)]
pub struct DocumentImageCache {
    entries: RefCell<HashMap<String, CachedFetch>>,
}

impl DocumentImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取得に失敗した参照が1つでもあるか。
    pub fn had_errors(&self) -> Option<String> {
        self.entries
            .borrow()
            .iter()
            .find_map(|(src, result)| result.as_ref().err().map(|e| format!("{src}: {e}")))
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
        // 直接呼ばず`ImageFetcher::resolve`を経由する。
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
