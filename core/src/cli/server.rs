//! `server`サブコマンド(HTTPサーバモード)。
//!
//! 設計は[0060](../../../docs/decisions/0060-http-server-mode.md)。
//!
//! * `POST /pdf?<CLIと同名のオプション>` + ボディは生HTML(決定1)
//! * クエリ文字列は**引数列へ機械変換して同じclapパーサに通す**
//!   ([0055](../../../docs/decisions/0055-cli-design.md)決定6)ので、
//!   CLIとサーバでオプションの解釈がずれない
//! * ローカル/リモートの参照は既定で禁止し、リクエストからは緩められない(決定3)

use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{CommandFactory, FromArgMatches};
use tiny_http::{Header, Request, Response, Server, StatusCode};

use crate::sink::{MemorySink, Sink};

use super::options::{Cli, ConvertArgs, ServerArgs};
use super::CliError;

/// クエリで指定させないオプション([0060]決定3)。
///
/// ローカルパスを取るもの・出力先・セキュリティ設定は、リクエストから
/// 変更できてはならない。
const DENIED_QUERY_KEYS: &[&str] = &[
    "font",
    "font-index",
    "gothic-font",
    "gothic-font-index",
    "serif-font",
    "serif-font-index",
    "mono-font",
    "mono-font-index",
    "output",
    "cover",
    "header-html",
    "footer-html",
    "user-style-sheet",
    "base-url",
    "allow",
    "enable-local-file-access",
    "disable-local-file-access",
    "allow-remote-assets",
    "log-level",
    "quiet",
];

pub fn run(args: &ServerArgs) -> Result<(), CliError> {
    let workers = args
        .workers
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let max_queue = args.max_queue.unwrap_or(workers * 4).max(1);

    let server = Server::http(&args.listen)
        .map_err(|e| CliError::Input(format!("{}で待ち受けられません: {e}", args.listen)))?;
    let addr = server
        .server_addr()
        .to_ip()
        .map(|a| a.to_string())
        .unwrap_or_else(|| args.listen.clone());
    // `--listen 127.0.0.1:0`で起動したときに実ポートを知るため、
    // 待ち受け開始を標準出力へ1行で出す(E2Eテストもこれを使う)。
    println!("listening on {addr}");

    let server = Arc::new(server);
    let shared = Arc::new(ServerContext {
        args: args.clone(),
        max_body_size: args.max_body_size,
    });

    // キュー溢れの判定用に、受理待ち件数をチャネルの長さで数える。
    let (tx, rx) = mpsc::sync_channel::<(Request, Instant)>(max_queue);
    let rx = Arc::new(std::sync::Mutex::new(rx));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let rx = Arc::clone(&rx);
        let shared = Arc::clone(&shared);
        let timeout = Duration::from_secs(args.timeout);
        handles.push(std::thread::spawn(move || loop {
            let next = {
                let guard = rx.lock().expect("受信キューのロックに失敗しました");
                guard.recv()
            };
            let Ok((request, queued_at)) = next else {
                break; // 送信側が閉じた = 終了
            };
            if queued_at.elapsed() > timeout {
                let _ = respond_text(request, 504, "キューでの待ち時間が--timeoutを超えました");
                continue;
            }
            handle_request(request, &shared);
        }));
    }

    for request in server.incoming_requests() {
        match tx.try_send((request, Instant::now())) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full((request, _))) => {
                let _ = respond_text(request, 503, "混雑しています(--max-queueを超えました)");
            }
            Err(mpsc::TrySendError::Disconnected(_)) => break,
        }
    }

    drop(tx);
    for handle in handles {
        let _ = handle.join();
    }
    Ok(())
}

struct ServerContext {
    args: ServerArgs,
    max_body_size: usize,
}

fn handle_request(mut request: Request, ctx: &ServerContext) {
    let url = request.url().to_string();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (url, String::new()),
    };
    let method = request.method().as_str().to_string();

    match (method.as_str(), path.as_str()) {
        ("GET", "/healthz") => {
            let _ = respond_text(request, 200, "ok");
        }
        ("GET", "/version") => {
            let _ = respond_text(
                request,
                200,
                &format!("sghtmltopdf {}", env!("CARGO_PKG_VERSION")),
            );
        }
        ("POST", "/pdf") if wants_chunked(&query) => {
            if let Err((status, message)) = respond_chunked(request, &query, ctx) {
                // respondより前に失敗した場合のみここへ来る(requestは消費済み
                // ではない)。ここでの`request`は使えないのでログだけ残す。
                eprintln!("エラー: {status} {message}");
            }
        }
        ("POST", "/pdf") => match render_request(&mut request, &query, ctx) {
            Ok(pdf) => {
                let header = Header::from_bytes(&b"Content-Type"[..], &b"application/pdf"[..])
                    .expect("固定のヘッダー値なので必ず作れる");
                let response = Response::from_data(pdf).with_header(header);
                let _ = request.respond(response);
            }
            Err((status, message)) => {
                let _ = respond_text(request, status, &message);
            }
        },
        (_, "/pdf") | (_, "/healthz") | (_, "/version") => {
            let _ = respond_text(request, 405, "このパスでは使えないメソッドです");
        }
        _ => {
            let _ = respond_text(request, 404, "not found");
        }
    }
}

/// `?stream=1`(chunkedでのストリーミング返却)が要求されているか。
fn wants_chunked(query: &str) -> bool {
    parse_query(query)
        .map(|pairs| {
            pairs.iter().any(|(key, value)| {
                key == "stream" && value.as_deref().map(is_true).unwrap_or(true)
            })
        })
        .unwrap_or(false)
}

/// `std::io::PipeWriter`へ書き出すSink。レンダリング側(push)と
/// HTTPレスポンス側(pull)をつなぐ([0060]決定2)。
struct PipeSink(std::io::PipeWriter);

impl Sink for PipeSink {
    type Output = ();
    type Error = std::io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(bytes)
    }

    fn finish(mut self) -> Result<Self::Output, Self::Error> {
        self.0.flush()
    }
}

/// `?stream=1`のときの応答。**ページが確定したそばからchunkedで流す**。
///
/// `tiny_http`の`Request`はボディ読み取り(`as_reader`が`&mut self`)と
/// 応答(`respond`が`self`)が排他なので、**入力は先に読み切ってから**
/// 出力を流す(入力ストリーミングとは同時に使えない、[0060]決定7)。
///
/// 既にヘッダを送ってしまうため、途中で失敗してもステータスは変えられない。
/// その場合はパイプを閉じるだけになり、クライアントには不完全なPDFが届く。
fn respond_chunked(
    mut request: Request,
    query: &str,
    ctx: &ServerContext,
) -> Result<(), (u16, String)> {
    let too_large = || {
        (
            413,
            format!("ボディが上限({}バイト)を超えています", ctx.max_body_size),
        )
    };
    if let Some(len) = request.body_length() {
        if len > ctx.max_body_size {
            let _ = respond_text(request, 413, &too_large().1);
            return Ok(());
        }
    }

    let stripped = strip_stream_key(query);
    let args = match build_convert_args(&stripped, &ctx.args) {
        Ok(args) => args,
        Err(message) => {
            let _ = respond_text(request, 400, &message);
            return Ok(());
        }
    };
    let fonts = ctx.args.font_specs();

    let mut html = Vec::new();
    let read = request
        .as_reader()
        .take(ctx.max_body_size as u64 + 1)
        .read_to_end(&mut html);
    match read {
        Ok(_) if html.len() > ctx.max_body_size => {
            let _ = respond_text(request, 413, &too_large().1);
            return Ok(());
        }
        Ok(_) if html.is_empty() => {
            let _ = respond_text(request, 400, "ボディにHTMLを入れてください");
            return Ok(());
        }
        Ok(_) => {}
        Err(e) => {
            let _ = respond_text(
                request,
                400,
                &format!("ボディの読み込みに失敗しました: {e}"),
            );
            return Ok(());
        }
    }

    let (pipe_reader, pipe_writer) = match std::io::pipe() {
        Ok(pair) => pair,
        Err(e) => {
            let _ = respond_text(request, 500, &format!("パイプを作れませんでした: {e}"));
            return Ok(());
        }
    };

    // レンダリングは別スレッドで走らせ、書き出したそばからパイプへ流す。
    // このスレッドはパイプの読み出し側をレスポンスとして返す。
    std::thread::spawn(move || {
        if let Err(e) = super::convert::render(
            &args,
            &fonts,
            std::io::Cursor::new(html),
            PipeSink(pipe_writer),
        ) {
            // ヘッダは送信済みなので、ここではログに残すことしかできない。
            eprintln!("エラー: ストリーミング返却の途中で失敗しました: {e}");
        }
    });

    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/pdf"[..])
        .expect("固定のヘッダー値なので必ず作れる");
    // `data_length`を`None`にするとchunked transfer encodingになる。
    let response = Response::new(StatusCode(200), vec![header], pipe_reader, None, None);
    let _ = request.respond(response);
    Ok(())
}

/// `stream`キーだけを取り除いたクエリ文字列(オプション解釈へは渡さない)。
fn strip_stream_key(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or(pair);
            key != "stream"
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// ボディを読みながらサイズ上限を数える`Read`ラッパー。
///
/// 上限を超えた時点でエラーを返し、`exceeded`を立てる。呼び出し側は
/// この印を見て413を返す(エンジンから返るエラーの文言に依存しない)。
struct LimitedReader<R> {
    inner: R,
    remaining: usize,
    read_any: bool,
    exceeded: bool,
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            // 1バイトでも余分に読めたら超過。
            let mut probe = [0u8; 1];
            if self.inner.read(&mut probe)? > 0 {
                self.exceeded = true;
                return Err(std::io::Error::other("body too large"));
            }
            return Ok(0);
        }
        let limit = buf.len().min(self.remaining);
        let read = self.inner.read(&mut buf[..limit])?;
        self.remaining -= read;
        if read > 0 {
            self.read_any = true;
        }
        Ok(read)
    }
}

/// 1リクエスト分の変換。エラーは(ステータス, メッセージ)で返す([0060]決定6)。
///
/// **ボディは読み切らずに`Engine::feed`へ流す**([0060]決定7)。
fn render_request(
    request: &mut Request,
    query: &str,
    ctx: &ServerContext,
) -> Result<Vec<u8>, (u16, String)> {
    let too_large = || {
        (
            413,
            format!("ボディが上限({}バイト)を超えています", ctx.max_body_size),
        )
    };

    // ボディ長が分かる場合は読む前に弾く。
    if let Some(len) = request.body_length() {
        if len > ctx.max_body_size {
            return Err(too_large());
        }
    }

    // クエリのパースはボディを読む前に済ませる(不正なら読まずに400)。
    let args = build_convert_args(query, &ctx.args).map_err(|e| (400, e))?;
    let fonts = ctx.args.font_specs();

    let mut reader = LimitedReader {
        inner: request.as_reader(),
        remaining: ctx.max_body_size,
        read_any: false,
        exceeded: false,
    };

    let sink = MemorySink::new();
    let result = super::convert::render_to_memory(&args, &fonts, &mut reader, sink);

    if reader.exceeded {
        return Err(too_large());
    }
    if !reader.read_any {
        return Err((400, "ボディにHTMLを入れてください".to_string()));
    }

    result.map_err(|e| match e {
        CliError::Usage(msg) => (400, msg),
        CliError::Input(msg) => (400, msg),
        CliError::Render(msg) => (500, msg),
    })
}

/// クエリ文字列をCLIの引数列へ変換し、同じclapパーサへ通す([0055]決定6)。
fn build_convert_args(query: &str, server: &ServerArgs) -> Result<ConvertArgs, String> {
    let mut argv: Vec<String> = vec!["sghtmltopdf".to_string()];
    // 入力はボディなので、位置引数にはstdinを表す`-`を置く(実際には読まない)。
    argv.push("-".to_string());
    argv.push("--output".to_string());
    argv.push("-".to_string());

    // サーバ起動時に固定するもの(リクエストからは変えられない、決定3)。
    for path in &server.font {
        argv.push("--font".to_string());
        argv.push(path.display().to_string());
    }
    for (flag, path) in [
        ("--gothic-font", server.gothic_font.as_ref()),
        ("--serif-font", server.serif_font.as_ref()),
        ("--mono-font", server.mono_font.as_ref()),
    ] {
        if let Some(path) = path {
            argv.push(flag.to_string());
            argv.push(path.display().to_string());
        }
    }
    if !server.enable_local_file_access {
        argv.push("--disable-local-file-access".to_string());
    }
    for dir in &server.allow {
        argv.push("--allow".to_string());
        argv.push(dir.display().to_string());
    }
    if server.allow_remote_assets {
        argv.push("--allow-remote-assets".to_string());
    }
    argv.push("--quiet".to_string());

    for (key, value) in parse_query(query)? {
        // 非対応オプションはCLIと同じ理由を返す([0055]決定5)。
        if let Some(reason) = super::unsupported::unsupported_reason(&format!("--{key}")) {
            return Err(format!("{key}は対応していません。{reason}"));
        }
        if DENIED_QUERY_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "{key}はリクエストからは指定できません(サーバ起動時のオプションで設定してください)"
            ));
        }
        match value {
            // 値なし / 真を表す値はフラグとして渡す。
            None => argv.push(format!("--{key}")),
            Some(v) if is_true(&v) => argv.push(format!("--{key}")),
            // 偽を表す値は「指定なし」と同じ。
            Some(v) if is_false(&v) => {}
            Some(v) => {
                argv.push(format!("--{key}"));
                argv.push(v);
            }
        }
    }

    let matches = Cli::command()
        .try_get_matches_from(&argv)
        .map_err(|e| e.to_string())?;
    let cli = Cli::from_arg_matches(&matches).map_err(|e| e.to_string())?;
    Ok(cli.convert)
}

fn is_true(value: &str) -> bool {
    matches!(value, "" | "1" | "true" | "yes" | "on")
}

fn is_false(value: &str) -> bool {
    matches!(value, "0" | "false" | "no" | "off")
}

/// `a=1&b&c=%E6%97%A5` をキーと値へ分解する(パーセントデコード込み)。
fn parse_query(query: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let mut out = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (pair, None),
        };
        let key = percent_decode(key)?;
        if key.is_empty() {
            continue;
        }
        let value = match value {
            Some(value) => Some(percent_decode(value)?),
            None => None,
        };
        out.push((key, value));
    }
    Ok(out)
}

/// `%XX`と`+`をデコードする(依存を増やさないための自前実装)。
fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .ok_or_else(|| format!("URLエンコードが壊れています: {text}"))?;
                let hex = std::str::from_utf8(hex)
                    .map_err(|_| format!("URLエンコードが壊れています: {text}"))?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("URLエンコードが壊れています: {text}"))?;
                out.push(byte);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| format!("UTF-8として解釈できません: {text}"))
}

fn respond_text(request: Request, status: u16, message: &str) -> std::io::Result<()> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
        .expect("固定のヘッダー値なので必ず作れる");
    let response = Response::from_string(message)
        .with_status_code(StatusCode(status))
        .with_header(header);
    request.respond(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_args() -> ServerArgs {
        ServerArgs {
            listen: "127.0.0.1:0".to_string(),
            workers: Some(1),
            max_queue: Some(1),
            max_body_size: 1024,
            timeout: 30,
            font: vec![std::path::PathBuf::from("/tmp/a.ttf")],
            gothic_font: None,
            serif_font: None,
            mono_font: None,
            enable_local_file_access: false,
            allow: Vec::new(),
            allow_remote_assets: false,
        }
    }

    #[test]
    fn query_pairs_are_percent_decoded() {
        let pairs = parse_query("a=1&b&c=%E6%97%A5%E6%9C%AC&d=x+y").unwrap();
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), Some("1".to_string())),
                ("b".to_string(), None),
                ("c".to_string(), Some("日本".to_string())),
                ("d".to_string(), Some("x y".to_string())),
            ]
        );
    }

    #[test]
    fn a_broken_escape_is_an_error() {
        assert!(parse_query("a=%zz").is_err());
        assert!(parse_query("a=%4").is_err());
    }

    #[test]
    fn query_options_reach_the_same_parser_as_the_cli() {
        let args = build_convert_args("page-size=A5&margin-top=20mm&toc", &server_args()).unwrap();
        let settings = args.page_settings().unwrap();
        assert_eq!(settings.size, crate::layout::PageSize::A5);
        assert!((settings.margin.top - 75.59).abs() < 0.1);
        assert!(args.toc);
    }

    #[test]
    fn boolean_values_are_understood() {
        let truthy = build_convert_args("grayscale=1&no-images=true", &server_args()).unwrap();
        assert!(truthy.grayscale);
        assert!(truthy.no_images);

        let falsy = build_convert_args("grayscale=0&no-images=false", &server_args()).unwrap();
        assert!(!falsy.grayscale);
        assert!(!falsy.no_images);
    }

    #[test]
    fn local_access_is_disabled_unless_the_server_enabled_it() {
        let args = build_convert_args("", &server_args()).unwrap();
        assert!(args.disable_local_file_access);
        assert!(!args.allow_remote_assets);

        let mut server = server_args();
        server.enable_local_file_access = true;
        server.allow_remote_assets = true;
        let args = build_convert_args("", &server).unwrap();
        assert!(!args.disable_local_file_access);
        assert!(args.allow_remote_assets);
    }

    #[test]
    fn denied_keys_are_rejected() {
        for key in [
            "font=/etc/passwd",
            "cover=/etc/passwd",
            "base-url=/etc",
            "output=/tmp/x.pdf",
        ] {
            let err = build_convert_args(key, &server_args()).unwrap_err();
            assert!(err.contains("指定できません"), "got: {err}");
        }
    }

    #[test]
    fn an_unknown_option_is_an_error() {
        assert!(build_convert_args("no-such-option=1", &server_args()).is_err());
    }
}
