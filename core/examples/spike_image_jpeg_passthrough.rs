//! T41スパイク: JPEGをHTTP経由で取得し、デコードせずにDCTDecodeフィルタとして
//! そのままPDFへ埋め込むPoC。
//!
//! 検証したいこと:
//! - `ureq`が同期(非async)APIでHTTP(S)フェッチを完結できるか。ローカルの
//!   ループバックHTTPサーバ(std::netのみ、外部ネットワーク接続は使わない)に
//!   対して実際にTCP往復させて確認する
//! - JPEGバイト列を一切デコードせず、SOF0/SOF2マーカーだけを読んで
//!   width/height/コンポーネント数を取り出せるか(画像デコードクレートを
//!   追加せずに済むかどうかの検証が主目的)
//! - `pdf-writer`の`ImageXObject`が`Filter::DctDecode`を受け付け、
//!   生のJPEGバイト列をそのままストリームとして埋め込めるか
//!
//! 実行: `cargo run --example spike_image_jpeg_passthrough`
//! (`tests/fixtures/images/spike_gradient.jpg`を使用。ベースラインJPEG)

use std::io::{Read, Write};
use std::net::TcpListener;

use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect as PdfRect, Ref};

const JPEG_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/images/spike_gradient.jpg"
);

/// SOF0(ベースライン)/SOF2(プログレッシブ)マーカーだけを読んでwidth/height/
/// コンポーネント数を取り出す。ピクセルデータのデコードは一切行わない。
fn parse_jpeg_dimensions(data: &[u8]) -> Option<(u16, u16, u8)> {
    if data.len() < 4 || data[0..2] != [0xFF, 0xD8] {
        return None; // SOIマーカーが無い
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15 がSOF系。
        // ここではベースライン(0xC0)とプログレッシブ(0xC2)のみ対応する。
        if marker == 0xC0 || marker == 0xC2 {
            let height = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let width = u16::from_be_bytes([data[i + 7], data[i + 8]]);
            let components = data[i + 9];
            return Some((width, height, components));
        }
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2; // 長さフィールドを持たないマーカー
            continue;
        }
        let segment_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 2 + segment_len;
    }
    None
}

/// 外部ネットワークに依存せず、ループバック上に1回だけ応答するHTTPサーバを起動する。
/// 実運用のフェッチ(T46)はこれと同じ`ureq`の同期API呼び出しで置き換わる想定。
fn spawn_single_response_server(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ループバックへのbindに失敗");
    let addr = listener.local_addr().unwrap();

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("接続の受理に失敗");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf); // リクエストの中身は読み捨てる(スパイクなので検証しない)

        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });

    format!("http://{addr}/spike.jpg")
}

fn main() {
    let jpeg_bytes = std::fs::read(JPEG_PATH).expect("テスト用JPEGフィクスチャの読み込みに失敗");
    let url = spawn_single_response_server(jpeg_bytes.clone());

    // T46で作る実際のfetch抽象化と同じく、ureqの同期APIで完結する
    // (asyncランタイムやスレッドプールを別途持ち込む必要が無い)。
    let fetched: Vec<u8> = ureq::get(&url)
        .call()
        .expect("HTTPフェッチに失敗")
        .body_mut()
        .read_to_vec()
        .expect("レスポンスボディの読み込みに失敗");

    assert_eq!(
        fetched, jpeg_bytes,
        "フェッチしたバイト列が元のJPEGと一致しない"
    );

    let (width, height, components) =
        parse_jpeg_dimensions(&fetched).expect("SOFマーカーからのサイズ解析に失敗");
    eprintln!("JPEG: {width}x{height}, components={components}");

    let mut ids = 0..;
    let mut next_id = || Ref::new(ids.next().unwrap() + 1);

    let catalog_id = next_id();
    let pages_tree_id = next_id();
    let page_id = next_id();
    let content_id = next_id();
    let image_id = next_id();

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(pages_tree_id);
    pdf.pages(pages_tree_id).kids([page_id]).count(1);

    let mut page = pdf.page(page_id);
    page.parent(pages_tree_id);
    page.media_box(PdfRect::new(0.0, 0.0, width as f32, height as f32));
    page.contents(content_id);
    page.resources().x_objects().pair(Name(b"Im0"), image_id);
    page.finish();

    let mut content = Content::new();
    // 画像1枚をページ全体に敷き詰める(cm行列でwidth/height分だけ拡大)。
    content.save_state();
    content.transform([width as f32, 0.0, 0.0, height as f32, 0.0, 0.0]);
    content.x_object(Name(b"Im0"));
    content.restore_state();
    pdf.stream(content_id, &content.finish());

    // ここが検証の核心: JPEGバイト列を一切デコードせず、そのままDCTDecode
    // フィルタのストリームとして埋め込む。
    let mut image = pdf.image_xobject(image_id, &fetched);
    image.width(width as i32);
    image.height(height as i32);
    match components {
        1 => image.color_space().device_gray(),
        3 => image.color_space().device_rgb(),
        4 => image.color_space().device_cmyk(),
        other => panic!("未対応のコンポーネント数: {other}"),
    }
    image.bits_per_component(8);
    image.filter(Filter::DctDecode);
    image.finish();

    let bytes = pdf.finish();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/spike_image_jpeg_passthrough.pdf");
    std::fs::write(&out, &bytes).unwrap();
    eprintln!(
        "wrote {} bytes to {} (embedded JPEG stayed {} bytes raw, no re-encode)",
        bytes.len(),
        out.display(),
        fetched.len()
    );
}
