//! T42スパイク: ureqの`Resolver`フックにSSRF対策(プライベート/loopback/
//! link-local等のIPを拒否するフィルタ)を差し込み、実際の`Agent`+リクエスト
//! 経路で機能することを検証するPoC。
//!
//! 検証したいこと:
//! - `ureq::unversioned::resolver::Resolver`トレイトは`Agent::with_parts`
//!   経由で差し替え可能で、リダイレクト追従時も毎回呼び直される
//!   (ソースコード確認: `ureq-3.3.0/src/run.rs`の`call_run`はredirectの
//!   loop 1周ごとに`connect()`→`agent.resolver.resolve()`を呼ぶ)。
//!   つまりこのフック1箇所で、初回アクセスだけでなく「公開URLとして許可した
//!   後にリダイレクトで内部IPへ」という古典的なSSRFバイパスも防げるはず
//! - DNSリバインディング対策: 名前解決の"結果のIP"だけを見てブロック判定
//!   すれば、ホスト名の文字列が公開ドメインらしく見えるかどうかに関係なく
//!   確実に拒否できる。これを「DNSが常に内部IPを返す」という最悪ケースの
//!   疑似リゾルバ(`AlwaysResolvesTo`)で実証する
//!
//! 実行: `cargo run --example spike_image_fetch_ssrf_guard`

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};

use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::{config::Config, Agent, Error};

/// プライベート/loopback/link-local(クラウドメタデータの169.254.169.254を
/// 含む)/マルチキャスト/未指定等、外部公開されるべきでないIPかどうかを
/// 判定する。IPv4-mapped IPv6(`::ffff:a.b.c.d`)は埋め込まれたIPv4側を
/// 再帰的に判定する(素通しするとIPv4側のフィルタを迂回できてしまうため)。
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local() // 169.254.0.0/16(クラウドメタデータ含む)
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
                || v6.is_unique_local() // fc00::/7
                || v6.is_unicast_link_local() // fe80::/10
        }
    }
}

/// 任意の`Resolver`をラップし、解決結果からブロック対象IPを除去する。
/// 1件も残らなければ`Error::HostNotFound`で拒否する
/// (「ブロックされた」と「そもそも存在しない」を呼び出し元から区別させない)。
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
    ) -> Result<ResolvedSocketAddrs, Error> {
        let addrs = self.inner.resolve(uri, config, timeout)?;
        let mut filtered: ResolvedSocketAddrs =
            ResolvedSocketAddrs::from_fn(|_| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        for addr in addrs.iter().filter(|a| !is_blocked_ip(a.ip())) {
            filtered.push(*addr);
        }
        if filtered.is_empty() {
            return Err(Error::HostNotFound);
        }
        Ok(filtered)
    }
}

/// DNSリバインディングの最悪ケースを模した疑似リゾルバ: 実際のホスト名を
/// 一切見ず、常に指定した(≒攻撃者が握っている)アドレスを返す。
#[derive(Debug)]
struct AlwaysResolvesTo(SocketAddr);

impl Resolver for AlwaysResolvesTo {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let mut addrs =
            ResolvedSocketAddrs::from_fn(|_| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        addrs.push(self.0);
        Ok(addrs)
    }
}

fn main() {
    // --- ケース1: 公開ホスト名のふりをしていても、標準の名前解決結果が
    // loopback/プライベートIPならブロックされること(現実の環境でループバック
    // サービスに直接アクセスするURLを想定)。
    let agent = Agent::with_parts(
        Config::default(),
        DefaultConnector::default(),
        PolicyResolver {
            inner: DefaultResolver::default(),
        },
    );
    let result = agent.get("http://127.0.0.1:1/should-be-blocked").call();
    match result {
        Err(Error::HostNotFound) => {
            eprintln!("[OK] loopback宛のリクエストはHostNotFoundとしてブロックされた")
        }
        other => panic!("loopback宛のリクエストがブロックされなかった: {other:?}"),
    }

    // --- ケース2: DNSリバインディングの最悪ケース。ホスト名の文字列が
    // 何であっても(ここでは`example.invalid`)、名前解決の"結果"が
    // プライベートIP(169.254.169.254、クラウドメタデータの定番アドレス)
    // であれば同じフィルタで拒否できること。
    let rebinding_agent = Agent::with_parts(
        Config::default(),
        DefaultConnector::default(),
        PolicyResolver {
            inner: AlwaysResolvesTo(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
                80,
            )),
        },
    );
    let result = rebinding_agent
        .get("http://example.invalid/latest/meta-data/")
        .call();
    match result {
        Err(Error::HostNotFound) => {
            eprintln!(
                "[OK] DNSリバインディングでクラウドメタデータIPへ誘導されるケースもブロックされた"
            )
        }
        other => panic!("メタデータIPへのリバインディングがブロックされなかった: {other:?}"),
    }

    // --- ケース3(対照実験): 公開的なIPは素通しされ、実際にTCP接続まで
    // 進むこと(=フィルタが過剰検知していないこと)。外部ネットワークには
    // 依存せず、ループバック上に立てた自前サーバを「グローバルIPのふり」
    // させることはできないため、ここでは`is_blocked_ip`単体の判定として
    // 確認する(実ネットワーク接続はしない)。
    let public_examples = [
        IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)), // example.com相当
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),       // 8.8.8.8 (Google Public DNS)
    ];
    for ip in public_examples {
        assert!(
            !is_blocked_ip(ip),
            "公開IPのはずが誤ってブロック対象と判定された: {ip}"
        );
    }
    eprintln!("[OK] 公開IPの例はブロック対象と判定されなかった(過剰検知していない)");

    // --- ケース4: 「リダイレクト追従時もresolve()が呼び直される」という
    // ソースコード読解(run.rsのcall_runがredirectのloopごとにconnect()経由で
    // resolver.resolve()を呼ぶ)を、実際にリダイレクトさせて実証する。
    // ここではポリシーフィルタ無しの素通しリゾルバに「見えたURIを記録する」
    // 機能だけ足したものを使い、302を1回挟んだ実リクエストで
    // resolve()が異なるauthority(host:port)で2回呼ばれることを確認する。
    // これが実証できれば、PolicyResolverを使った場合に「最初のURLは許可
    // されたが302先が内部IPだった」というリダイレクト経由のSSRFも、
    // 2回目のresolve()呼び出しで同じフィルタにより拒否されることが担保される。
    let seen_authorities: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let redirect_target_addr = spawn_plain_ok_server();
    let redirect_target = format!("http://127.0.0.1:{}/final", redirect_target_addr.port());
    let entry_addr = spawn_redirecting_server(redirect_target);

    let counting_agent = Agent::with_parts(
        Config::default(),
        DefaultConnector::default(),
        LoggingResolver {
            inner: DefaultResolver::default(),
            seen: seen_authorities.clone(),
        },
    );
    let response = counting_agent
        .get(format!("http://127.0.0.1:{}/start", entry_addr.port()))
        .call()
        .expect("リダイレクトを挟んだリクエストが失敗した");
    assert_eq!(response.status(), 200);

    let seen = seen_authorities.lock().unwrap();
    assert_eq!(
        seen.len(),
        2,
        "resolve()が2回(リダイレクト前後で1回ずつ)呼ばれることを期待したが実際は{}回だった: {seen:?}",
        seen.len()
    );
    assert_ne!(
        seen[0], seen[1],
        "リダイレクト前後で異なるhost:portに対してresolve()が呼ばれることを期待した"
    );
    eprintln!("[OK] リダイレクト追従で resolve() が呼び直されることを確認した: {seen:?}");

    eprintln!("すべてのSSRF対策シナリオを確認できた");
}

/// `Resolver::resolve`に渡された`Uri`のauthority(host:port)を記録するだけの
/// 素通しラッパー。ケース4で「リダイレクトのたびに呼び直されるか」を
/// 実証するために使う(SSRFフィルタ自体は掛けない)。
#[derive(Debug)]
struct LoggingResolver<R> {
    inner: R,
    seen: Arc<Mutex<Vec<String>>>,
}

impl<R: Resolver> Resolver for LoggingResolver<R> {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        self.seen
            .lock()
            .unwrap()
            .push(uri.authority().map(|a| a.to_string()).unwrap_or_default());
        self.inner.resolve(uri, config, timeout)
    }
}

/// 1回だけ`200 OK`を返すループバックサーバを起動し、bindしたアドレスを返す。
fn spawn_plain_ok_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ループバックへのbindに失敗");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("接続の受理に失敗");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    addr
}

/// 1回だけ`302 Found`(指定URLへのLocationヘッダ付き)を返すループバック
/// サーバを起動し、bindしたアドレスを返す。
fn spawn_redirecting_server(location: String) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ループバックへのbindに失敗");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("接続の受理に失敗");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    addr
}
