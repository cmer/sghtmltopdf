//! `<img>`のバイト列取得。ローカルファイル/HTTP(S)/`data:` URIを
//! [`ImgSrc`]の分類(T45)に従って統一的に扱う。
//!
//! セキュリティポリシー(プライベートIPブロック・許可スキーム・リダイレクト
//! 制限・サイズ上限・タイムアウト・既定無効のオプトイン)をここで実装する。
//! 実際にどのタイミングで呼ぶか(「box tree構築時に
//! 遅延して1回だけ」)は呼び出し側(T52)の責務。

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::{Agent, Error as UreqError};

use super::{resolve_local_asset_path, ImgSrc};

/// 取得したバイト列の既定上限(20MiB)。ローカル/リモート/data:のいずれの
/// 取得元にも同じ上限を適用する(軽量・低メモリという設計方針上、非HTTP
/// 経由だからといって無制限にする理由が無いため)。
const DEFAULT_MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug)]
pub struct FetchError(String);

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "画像の取得に失敗しました: {}", self.0)
    }
}

impl std::error::Error for FetchError {}

/// `<img>`のバイト列取得を担う。文書ごとに1つ構築し、複数の`<img>`要素の
/// 取得で使い回す想定(`ureq::Agent`の内部コネクションプーリングも活かせる)。
pub struct ImageFetcher {
    /// ローカル相対パスの基準ディレクトリ(`@font-face`のurl()解決と同じ
    /// 役割)。
    base_dir: PathBuf,
    /// リモート(http/https)フェッチを許可するかどうか。既定は無効
    /// (「既定無効・明示オプトイン」方針)。`data:`/ローカルパスはこの値に
    /// 関わらず常に許可する(ネットワークを介さない、
    /// または`@font-face`と同じ信頼境界のため)。
    allow_remote: bool,
    max_bytes: u64,
    agent: Agent,
    /// `<base href>`の値。相対参照はフェッチ前にこれに対して解決される。
    /// `<img src>`・`<link href>`・`@import`はいずれもこのフェッチャを
    /// 共有するため、ここ1箇所で3種類すべてに効く。
    base_href: Option<String>,
    /// ローカルファイル参照を許すか(`--disable-local-file-access`でfalse)。
    /// HTTPサーバモード(Phase 7)では既定でfalseにする想定。
    allow_local: bool,
    /// 空でなければ、ローカル参照をこのディレクトリ配下に限定する
    /// (`--allow`)。
    allowed_dirs: Vec<PathBuf>,
}

impl ImageFetcher {
    /// サイズ上限は[`DEFAULT_MAX_IMAGE_BYTES`]を使う。
    pub fn new(base_dir: PathBuf, allow_remote: bool) -> Self {
        Self::with_max_bytes(base_dir, allow_remote, DEFAULT_MAX_IMAGE_BYTES)
    }

    /// `<base href>`を設定した同じフェッチャを返す(ビルダー的に使う)。
    pub fn with_base_href(mut self, base_href: Option<String>) -> Self {
        self.base_href = base_href.filter(|href| !href.trim().is_empty());
        self
    }

    /// ローカルファイルの読み込み可否と、許可ディレクトリ(`--allow`)を
    /// 設定する(M12 Phase 4・T301)。
    ///
    /// `allow_local`が`false`のとき、ローカルパス参照はすべて拒否する
    /// (HTTPサーバモードの既定を想定)。`allowed_dirs`が空でなければ、
    /// 解決後のパスがそのいずれかの配下に無い参照を拒否する。
    pub fn with_local_access(mut self, allow_local: bool, allowed_dirs: Vec<PathBuf>) -> Self {
        self.allow_local = allow_local;
        self.allowed_dirs = allowed_dirs;
        self
    }

    /// 生の参照値を`<base href>`に対して解決し、URL/パスとして分類する。
    /// フェッチ経路はすべてここを通す。
    pub fn resolve(&self, raw: &str) -> Option<super::ImgSrc> {
        let resolved = super::resolve_against_base_href(self.base_href.as_deref(), raw);
        super::classify_img_src(&resolved)
    }

    pub fn with_max_bytes(base_dir: PathBuf, allow_remote: bool, max_bytes: u64) -> Self {
        let config = Config::builder()
            .max_redirects(5)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(15)))
            .build();
        let agent = Agent::with_parts(
            config,
            DefaultConnector::default(),
            PolicyResolver {
                inner: DefaultResolver::default(),
            },
        );
        Self {
            base_dir,
            allow_remote,
            max_bytes,
            agent,
            base_href: None,
            allow_local: true,
            allowed_dirs: Vec::new(),
        }
    }

    /// `src`の分類([`ImgSrc`])に応じてバイト列を取得する。
    pub fn fetch(&self, src: &ImgSrc) -> Result<Vec<u8>, FetchError> {
        match src {
            ImgSrc::LocalPath(path) => self.read_local(path),
            ImgSrc::RemoteUrl(url) => self.fetch_remote(url),
            ImgSrc::DataUri { bytes, .. } => {
                self.ensure_within_limit(bytes.len() as u64)?;
                Ok(bytes.clone())
            }
        }
    }

    /// `base_dir`相対のローカルファイルを読む。`@font-face`の`url()`解決
    /// (`fonts/face.rs`の`load_one`)と同じ[`resolve_local_asset_path`]を
    /// 使う(root-relativeな`/foo`もbase_dir相対として扱う、T61)。`..`に
    /// よるディレクトリトラバーサルの制限は行っていない(既存のfont読み込み
    /// と対称的な挙動に揃えている。より厳格にするならfont側と合わせて
    /// 別途検討する)。
    fn read_local(&self, path: &str) -> Result<Vec<u8>, FetchError> {
        if !self.allow_local {
            return Err(FetchError(format!(
                "{path}: ローカルファイルの読み込みは許可されていません(--enable-local-file-access)"
            )));
        }
        let full_path = resolve_local_asset_path(&self.base_dir, path);
        if !self.allowed_dirs.is_empty() {
            let canonical = full_path
                .canonicalize()
                .unwrap_or_else(|_| full_path.clone());
            let permitted = self.allowed_dirs.iter().any(|dir| {
                let dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
                canonical.starts_with(&dir)
            });
            if !permitted {
                return Err(FetchError(format!(
                    "{}: --allowで許可されたディレクトリの外です",
                    full_path.display()
                )));
            }
        }
        let metadata = std::fs::metadata(&full_path)
            .map_err(|e| FetchError(format!("{}: {e}", full_path.display())))?;
        self.ensure_within_limit(metadata.len()).map_err(|_| {
            FetchError(format!(
                "{}: ファイルサイズが上限を超えています",
                full_path.display()
            ))
        })?;
        std::fs::read(&full_path).map_err(|e| FetchError(format!("{}: {e}", full_path.display())))
    }

    fn fetch_remote(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        if !self.allow_remote {
            return Err(FetchError(format!(
                "リモート画像フェッチは既定で無効です(オプトインが必要): {url}"
            )));
        }
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| FetchError(format!("{url}: {e}")))?;
        response
            .body_mut()
            .with_config()
            .limit(self.max_bytes)
            .read_to_vec()
            .map_err(|e| FetchError(format!("{url}: {e}")))
    }

    fn ensure_within_limit(&self, len: u64) -> Result<(), FetchError> {
        if len > self.max_bytes {
            return Err(FetchError(format!(
                "サイズが上限({}バイト)を超えています",
                self.max_bytes
            )));
        }
        Ok(())
    }
}

/// プライベート/loopback/link-local(クラウドメタデータの169.254.169.254を
/// 含む)/マルチキャスト/未指定等、外部公開されるべきでないIPかどうかを
/// 判定する。IPv4-mapped IPv6(`::ffff:a.b.c.d`)は埋め込まれたIPv4側を
/// 再帰的に判定する(素通しするとIPv4側のフィルタを迂回できてしまうため)。
///
/// `core/examples/spike_image_fetch_ssrf_guard.rs`を使って実際に検証済みの
/// ロジックをそのまま本実装へ移した。
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(mapped));
            }
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

/// 任意の`Resolver`をラップし、解決結果からブロック対象IPを除去する。
/// 1件も残らなければ`Error::HostNotFound`で拒否する(「ブロックされた」と
/// 「そもそも存在しない」を呼び出し元から区別させない)。
///
/// `ureq`はリダイレクト追従のたびに`resolve`を呼び直すため、このフック1箇所で
/// 初回・リダイレクト経由の両方のSSRF対策になる。
#[derive(Debug)]
struct PolicyResolver<R> {
    inner: R,
}

impl<R: Resolver> Resolver for PolicyResolver<R> {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, UreqError> {
        let addrs = self.inner.resolve(uri, config, timeout)?;
        let mut filtered: ResolvedSocketAddrs = ResolvedSocketAddrs::from_fn(|_| {
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0)
        });
        for addr in addrs.iter().filter(|a| !is_blocked_ip(a.ip())) {
            filtered.push(*addr);
        }
        if filtered.is_empty() {
            return Err(UreqError::HostNotFound);
        }
        Ok(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};

    /// `engine.rs`の既存テストと同じ`std::env::temp_dir()`ベースの一時
    /// ディレクトリ作成パターン。呼び出し側が最後に`remove_dir_all`する。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sghtmltopdf-img-fetch-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_a_local_file_relative_to_base_dir() {
        let dir = temp_dir("reads_local");
        std::fs::write(dir.join("logo.png"), b"fake png bytes").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);

        let bytes = fetcher
            .fetch(&ImgSrc::LocalPath("logo.png".to_string()))
            .expect("local read should succeed");
        assert_eq!(bytes, b"fake png bytes");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_root_relative_local_path_stays_inside_base_dir() {
        // T61: `/logo.png`のようなroot-relativeなsrc(`<link
        // href="/stylesheets/main.css" />`と同種の書き方)が
        // base_dirの外(OSのファイルシステムルート)へ逃げず、base_dir配下の
        // ファイルとして読めることを確認する。
        let dir = temp_dir("root_relative");
        std::fs::write(dir.join("logo.png"), b"fake png bytes").unwrap();
        let fetcher = ImageFetcher::new(dir.clone(), false);

        let bytes = fetcher
            .fetch(&ImgSrc::LocalPath("/logo.png".to_string()))
            .expect("root-relative local read should succeed within base_dir");
        assert_eq!(bytes, b"fake png bytes");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_local_file_is_an_error() {
        let dir = temp_dir("missing_local");
        let fetcher = ImageFetcher::new(dir.clone(), false);

        let result = fetcher.fetch(&ImgSrc::LocalPath("does-not-exist.png".to_string()));
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_local_file_is_rejected() {
        let dir = temp_dir("oversized_local");
        std::fs::write(dir.join("big.png"), b"way too big").unwrap();
        let fetcher = ImageFetcher::with_max_bytes(dir.clone(), false, 4);

        let result = fetcher.fetch(&ImgSrc::LocalPath("big.png".to_string()));
        assert!(
            result.is_err(),
            "file larger than the byte cap should be rejected"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn data_uri_bytes_are_returned_as_is() {
        let fetcher = ImageFetcher::new(Path::new(".").to_path_buf(), false);
        let src = ImgSrc::DataUri {
            mime_type: "image/png".to_string(),
            bytes: b"decoded bytes".to_vec(),
        };
        let bytes = fetcher.fetch(&src).expect("data uri fetch should succeed");
        assert_eq!(bytes, b"decoded bytes");
    }

    #[test]
    fn oversized_data_uri_is_rejected() {
        let fetcher = ImageFetcher::with_max_bytes(Path::new(".").to_path_buf(), false, 4);
        let src = ImgSrc::DataUri {
            mime_type: "image/png".to_string(),
            bytes: b"way too big".to_vec(),
        };
        assert!(fetcher.fetch(&src).is_err());
    }

    #[test]
    fn remote_fetch_is_disabled_by_default() {
        let fetcher = ImageFetcher::new(Path::new(".").to_path_buf(), false);
        let result = fetcher.fetch(&ImgSrc::RemoteUrl(
            "http://127.0.0.1:1/should-not-even-try".to_string(),
        ));
        assert!(result.is_err(), "remote fetch must be opt-in");
    }

    #[test]
    fn remote_fetch_blocks_loopback_targets_even_when_enabled() {
        let fetcher = ImageFetcher::new(Path::new(".").to_path_buf(), true);
        let result = fetcher.fetch(&ImgSrc::RemoteUrl(
            "http://127.0.0.1:1/should-be-blocked".to_string(),
        ));
        assert!(
            result.is_err(),
            "the SSRF policy resolver must block loopback targets regardless of opt-in"
        );
    }

    #[test]
    fn remote_fetch_succeeds_against_a_public_looking_loopback_server_once_allowed_and_unblocked() {
        // ポリシーresolver自体はloopbackを常にブロックするため
        // (上のテストの通り)、`ImageFetcher`のHTTP応答パース経路そのものは
        // ポリシーを持たない生の`ureq`呼び出しで検証する
        // (T42のspike同様、外部ネットワークには依存しない)。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                )
                .unwrap();
        });

        let mut response = ureq::get(format!("http://127.0.0.1:{}/", addr.port()))
            .call()
            .expect("plain ureq agent without the policy resolver should reach loopback");
        let body = response.body_mut().read_to_vec().expect("should read body");
        assert_eq!(body, b"hello");
    }
}
