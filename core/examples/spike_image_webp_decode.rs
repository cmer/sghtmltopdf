//! 追加スパイク: WebPを`image`crate(webp機能のみ有効化)でデコードし、
//! PNGスパイク(`spike_image_png_decode.rs`)と同じRGB本体+SMask分離方式で
//! PDFへ埋め込むPoC。
//!
//! なぜ`image`crateなのか: PNG/JPEGは専用クレート(`png`単体、JPEGはデコード
//! 無しのDCTDecodeパススルー)で足りるが、WebPには相当する軽量単体クレートが
//! 無い(`libwebp-sys`はCライブラリバインディングでPure Rustではない)。
//! `image`crateはデフォルト機能だとAV1/TIFF/GIF等が芋づる式に付いてくるため
//! 全体は不採用としたが、`default-features = false, features = ["webp"]`に
//! 絞ると追加は`image`/`image-webp`/`quick-error`/`moxcms`/`pxfm`等
//! 9crateのみで、AV1エンコーダ一式(`rav1e`等)は一切付いてこないことを
//! 確認済み。既存の`png`crateとも依存が大きく
//! 重複する(flate2系)ため実質的な純増は小さい
//!
//! 実行: `cargo run --example spike_image_webp_decode`
//! (`tests/fixtures/images/spike_gradient_alpha.webp`を使用。右半分が半透明の
//! ロスレスWebP)

use std::io::BufReader;

use image::codecs::webp::WebPDecoder;
use image::{ColorType, ImageDecoder};

use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect as PdfRect, Ref};

const WEBP_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/images/spike_gradient_alpha.webp"
);

fn main() {
    let file = BufReader::new(
        std::fs::File::open(WEBP_PATH).expect("テスト用WebPフィクスチャの読み込みに失敗"),
    );
    let decoder = WebPDecoder::new(file).expect("WebPヘッダの読み込みに失敗");

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    eprintln!("WebP: {width}x{height}, color_type={color_type:?}");

    let mut buf = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut buf)
        .expect("フレームのデコードに失敗");

    // PNGスパイクと同じく、色本体とアルファチャンネルを分離する。
    let (rgb, alpha): (Vec<u8>, Option<Vec<u8>>) = match color_type {
        ColorType::Rgb8 => (buf, None),
        ColorType::Rgba8 => {
            let pixel_count = (width * height) as usize;
            let mut rgb = Vec::with_capacity(pixel_count * 3);
            let mut alpha = Vec::with_capacity(pixel_count);
            for px in buf.as_chunks::<4>().0 {
                rgb.extend_from_slice(&px[0..3]);
                alpha.push(px[3]);
            }
            (rgb, Some(alpha))
        }
        other => panic!("このスパイクでは未検証のcolor_type: {other:?}"),
    };

    let mut ids = 0..;
    let mut next_id = || Ref::new(ids.next().unwrap() + 1);

    let catalog_id = next_id();
    let pages_tree_id = next_id();
    let page_id = next_id();
    let content_id = next_id();
    let image_id = next_id();
    let smask_id = next_id();

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
    content.save_state();
    content.transform([width as f32, 0.0, 0.0, height as f32, 0.0, 0.0]);
    content.x_object(Name(b"Im0"));
    content.restore_state();
    pdf.stream(content_id, &content.finish());

    if let Some(alpha) = &alpha {
        let compressed = deflate(alpha);
        let mut smask = pdf.image_xobject(smask_id, &compressed);
        smask.width(width as i32);
        smask.height(height as i32);
        smask.color_space().device_gray();
        smask.bits_per_component(8);
        smask.filter(Filter::FlateDecode);
        smask.finish();
    }

    let compressed_rgb = deflate(&rgb);
    let mut image = pdf.image_xobject(image_id, &compressed_rgb);
    image.width(width as i32);
    image.height(height as i32);
    image.color_space().device_rgb();
    image.bits_per_component(8);
    image.filter(Filter::FlateDecode);
    if alpha.is_some() {
        image.s_mask(smask_id);
    }
    image.finish();

    let bytes = pdf.finish();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/spike_image_webp_decode.pdf");
    std::fs::write(&out, &bytes).unwrap();
    eprintln!(
        "wrote {} bytes to {} (alpha channel present: {})",
        bytes.len(),
        out.display(),
        alpha.is_some()
    );
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}
