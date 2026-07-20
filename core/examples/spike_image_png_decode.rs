//! T41スパイク: `png`クレートでPNG(パレット/インターレース/透過を含みうる)を
//! 生ピクセルへデコードし、色本体とアルファチャンネルを分離してPDFへ埋め込むPoC。
//!
//! JPEGと違い、PNGは一般にDCTDecodeのような生バイト列パススルーができない
//! (インターレース・パレット・任意ビット深度をPDF側のPredictorだけでは
//! 再現しきれないため)。そのため`png`クレートでの完全デコード
//! (インターレース解除・パレット展開・8bit正規化まで含む)が必要になる。
//!
//! 検証したいこと:
//! - `png::Transformations::normalize_to_color8()`で、パレット/低ビット深度/
//!   インターレースを問わず8bit RGB(A)へ正規化してデコードできるか
//! - デコード結果からアルファチャンネルを分離し、本体はDeviceRGB+FlateDecode、
//!   アルファは別XObjectのDeviceGray+FlateDecodeとして`/SMask`で紐付けられるか
//!
//! 実行: `cargo run --example spike_image_png_decode`
//! (`tests/fixtures/images/spike_gradient_alpha.png`を使用。右半分が半透明)

use png::{ColorType, Transformations};

use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect as PdfRect, Ref};

const PNG_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/images/spike_gradient_alpha.png"
);

fn main() {
    let file = std::io::BufReader::new(
        std::fs::File::open(PNG_PATH).expect("テスト用PNGフィクスチャの読み込みに失敗"),
    );
    let mut decoder = png::Decoder::new(file);
    // パレット展開・低ビット深度の8bit化・tRNSのアルファ化をまとめて有効にする。
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("PNGヘッダの読み込みに失敗");

    let mut buf = vec![0u8; reader.output_buffer_size().expect("frame countが不明")];
    let info = reader
        .next_frame(&mut buf)
        .expect("フレームのデコードに失敗");
    let (width, height) = (info.width, info.height);
    eprintln!(
        "PNG: {width}x{height}, color_type={:?}, bit_depth={:?}",
        info.color_type, info.bit_depth
    );

    // 正規化後はGrayscale/GrayscaleAlpha/Rgb/Rgbaのいずれかになる(パレットは
    // normalize_to_color8()でRGB(A)へ展開済みのため、ここでIndexedは出てこない)。
    let (rgb, alpha): (Vec<u8>, Option<Vec<u8>>) = match info.color_type {
        ColorType::Rgb => (buf, None),
        ColorType::Rgba => {
            let pixel_count = (width * height) as usize;
            let mut rgb = Vec::with_capacity(pixel_count * 3);
            let mut alpha = Vec::with_capacity(pixel_count);
            for px in buf.chunks_exact(4) {
                rgb.extend_from_slice(&px[0..3]);
                alpha.push(px[3]);
            }
            (rgb, Some(alpha))
        }
        ColorType::Grayscale => {
            let rgb = buf.iter().flat_map(|&g| [g, g, g]).collect();
            (rgb, None)
        }
        ColorType::GrayscaleAlpha => {
            let pixel_count = (width * height) as usize;
            let mut rgb = Vec::with_capacity(pixel_count * 3);
            let mut alpha = Vec::with_capacity(pixel_count);
            for px in buf.chunks_exact(2) {
                rgb.extend_from_slice(&[px[0], px[0], px[0]]);
                alpha.push(px[1]);
            }
            (rgb, Some(alpha))
        }
        ColorType::Indexed => unreachable!("normalize_to_color8()によりIndexedはRGB(A)へ展開済み"),
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

    // アルファチャンネルがあれば先に独立したDeviceGray SMaskとして書き出す
    // (本体側から/SMaskで参照する)。ここもFlateDecode(zlib圧縮)で圧縮する。
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
        .join("../target/spike_image_png_decode.pdf");
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
