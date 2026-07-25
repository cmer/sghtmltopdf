//! 画像バイト列(JPEG/PNG/WebP)をPDF Image XObject埋め込み用データへ変換する。
//!
//! `core/src/img/`(URL解決・フェッチ・キャッシュ)が返す生バイト列を受け取り、
//! フォーマットをマジックバイトで判別してデコードする。JPEGは
//! [0012](../../../docs/decisions/0012-image-embedding-crates.md)の決定
//! 通りデコードせず、SOFマーカーからwidth/height/コンポーネント数だけを
//! 読んでDCTDecodeフィルタでそのまま埋め込む(`core/examples/
//! spike_image_jpeg_passthrough.rs`で検証済みの方式)。PNG/WebPはそれぞれ
//! `png`クレート/`image`クレート(webp機能のみ)でフルデコードし、
//! アルファチャンネルがあれば色本体から分離して別XObjectの`/SMask`とする
//! (`spike_image_png_decode.rs`/`spike_image_webp_decode.rs`で検証済み)。
//!
//! ここで作る[`PreparedImage`]は`pdf-writer`の`Ref`をまだ持たない
//! (実際のRef割当・`Chunk`書き出しはbox tree構築時にsrcごとに1回行う
//! T51以降の責務)。

use std::io::Cursor;

use pdf_writer::{Filter, Finish};
use png::Transformations;

use super::font::deflate;

/// フォーマット判別からPDF埋め込み用データへの変換までの過程で起きた失敗。
#[derive(Debug)]
pub struct ImageDecodeError(String);

impl std::fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "画像のデコードに失敗しました: {}", self.0)
    }
}

impl std::error::Error for ImageDecodeError {}

/// PDF Image XObjectとして埋め込む直前のデータ(1ストリーム分)。
#[derive(Debug, Clone)]
pub struct ImagePlane {
    pub data: Vec<u8>,
    pub filter: Filter,
    pub color_space: PlaneColorSpace,
    pub bits_per_component: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneColorSpace {
    Gray,
    Rgb,
    Cmyk,
}

/// デコード結果。`alpha`があれば`color`の`/SMask`として別XObjectに書き出す。
#[derive(Debug, Clone)]
pub struct PreparedImage {
    pub width: u32,
    pub height: u32,
    pub color: ImagePlane,
    pub alpha: Option<ImagePlane>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFormat {
    Jpeg,
    Png,
    WebP,
}

/// マジックバイトからフォーマットを判別する。宣言された`Content-Type`/
/// `data:`のmime typeは信用せず、実際のバイト列だけで判定する。
fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(ImageFormat::Png)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// 画像バイト列をフォーマット判別した上でデコードし、PDF埋め込み用データへ
/// 変換する。
pub fn decode_image(bytes: &[u8]) -> Result<PreparedImage, ImageDecodeError> {
    match sniff_format(bytes) {
        Some(ImageFormat::Jpeg) => decode_jpeg(bytes),
        Some(ImageFormat::Png) => decode_png(bytes),
        Some(ImageFormat::WebP) => decode_webp(bytes),
        None => Err(ImageDecodeError(
            "対応していない画像フォーマットです(JPEG/PNG/WebPのいずれでもありません)".to_string(),
        )),
    }
}

/// SOF0(ベースライン)/SOF2(プログレッシブ)マーカーだけを読んでwidth/height/
/// コンポーネント数を取り出す。ピクセルデータのデコードは一切行わない
/// (`spike_image_jpeg_passthrough.rs`の`parse_jpeg_dimensions`と同一)。
fn parse_jpeg_dimensions(data: &[u8]) -> Option<(u16, u16, u8)> {
    if data.len() < 4 || data[0..2] != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xC0 || marker == 0xC2 {
            let height = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let width = u16::from_be_bytes([data[i + 7], data[i + 8]]);
            let components = data[i + 9];
            return Some((width, height, components));
        }
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let segment_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 2 + segment_len;
    }
    None
}

fn decode_jpeg(bytes: &[u8]) -> Result<PreparedImage, ImageDecodeError> {
    let (width, height, components) = parse_jpeg_dimensions(bytes)
        .ok_or_else(|| ImageDecodeError("SOFマーカーが見つかりません".to_string()))?;
    let color_space = match components {
        1 => PlaneColorSpace::Gray,
        3 => PlaneColorSpace::Rgb,
        4 => PlaneColorSpace::Cmyk,
        other => {
            return Err(ImageDecodeError(format!(
                "未対応のJPEGコンポーネント数です: {other}"
            )))
        }
    };
    Ok(PreparedImage {
        width: width as u32,
        height: height as u32,
        color: ImagePlane {
            data: bytes.to_vec(),
            filter: Filter::DctDecode,
            color_space,
            bits_per_component: 8,
        },
        alpha: None,
    })
}

fn decode_png(bytes: &[u8]) -> Result<PreparedImage, ImageDecodeError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|e| ImageDecodeError(e.to_string()))?;

    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| ImageDecodeError("frame情報が取得できません".to_string()))?;
    let mut buf = vec![0u8; buffer_size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| ImageDecodeError(e.to_string()))?;
    let (width, height) = (info.width, info.height);

    // normalize_to_color8()によりIndexedはRGB(A)へ展開済みのため、ここで
    // 出てくるのはGrayscale/GrayscaleAlpha/Rgb/Rgbaのいずれか。グレースケール
    // 画像をRGBへ水増しする必要は無い(3倍のバイト数になってしまう)ため、
    // 元のチャンネル構成を保ったままcolor_spaceだけ変える。
    let (color_bytes, color_space, alpha) = match info.color_type {
        png::ColorType::Rgb => (buf, PlaneColorSpace::Rgb, None),
        png::ColorType::Grayscale => (buf, PlaneColorSpace::Gray, None),
        png::ColorType::Rgba => {
            let (color, alpha) = split_interleaved_alpha(&buf, 4);
            (color, PlaneColorSpace::Rgb, Some(alpha))
        }
        png::ColorType::GrayscaleAlpha => {
            let (color, alpha) = split_interleaved_alpha(&buf, 2);
            (color, PlaneColorSpace::Gray, Some(alpha))
        }
        png::ColorType::Indexed => {
            return Err(ImageDecodeError(
                "normalize_to_color8()の後にIndexedが残るのは想定外です".to_string(),
            ))
        }
    };

    Ok(PreparedImage {
        width,
        height,
        color: ImagePlane {
            data: deflate(&color_bytes),
            filter: Filter::FlateDecode,
            color_space,
            bits_per_component: 8,
        },
        alpha: alpha.map(|a| ImagePlane {
            data: deflate(&a),
            filter: Filter::FlateDecode,
            color_space: PlaneColorSpace::Gray,
            bits_per_component: 8,
        }),
    })
}

fn decode_webp(bytes: &[u8]) -> Result<PreparedImage, ImageDecodeError> {
    use image::{ColorType, ImageDecoder};

    let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
        .map_err(|e| ImageDecodeError(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    let mut buf = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut buf)
        .map_err(|e| ImageDecodeError(e.to_string()))?;

    let (color_bytes, alpha) = match color_type {
        ColorType::Rgb8 => (buf, None),
        ColorType::Rgba8 => {
            let (color, alpha) = split_interleaved_alpha(&buf, 4);
            (color, Some(alpha))
        }
        other => {
            return Err(ImageDecodeError(format!(
                "未対応のWebP color_typeです: {other:?}"
            )))
        }
    };

    Ok(PreparedImage {
        width,
        height,
        color: ImagePlane {
            data: deflate(&color_bytes),
            filter: Filter::FlateDecode,
            color_space: PlaneColorSpace::Rgb,
            bits_per_component: 8,
        },
        alpha: alpha.map(|a| ImagePlane {
            data: deflate(&a),
            filter: Filter::FlateDecode,
            color_space: PlaneColorSpace::Gray,
            bits_per_component: 8,
        }),
    })
}

/// `stride`(3+1=4、または1+1=2)おきにインターリーブされたバッファから、
/// 最後の1チャンネル(アルファ)を分離する。残りが色本体になる。
fn split_interleaved_alpha(buf: &[u8], stride: usize) -> (Vec<u8>, Vec<u8>) {
    let pixel_count = buf.len() / stride;
    let mut color = Vec::with_capacity(pixel_count * (stride - 1));
    let mut alpha = Vec::with_capacity(pixel_count);
    for px in buf.chunks_exact(stride) {
        color.extend_from_slice(&px[..stride - 1]);
        alpha.push(px[stride - 1]);
    }
    (color, alpha)
}

// --- 文書内での「取得→デコード」結果の共有、およびPDF Image XObjectとしての
// 書き出し(T51〜T54)。 ---
//
// [0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)は
// 「文書内キャッシュはRef+内在サイズだけを持つ」設計を示していたが、実装を
// 進める過程で「デコード結果(`PreparedImage`)自体を`Rc`で共有し、Ref割当・
// 実際のXObject書き出しはPDFエンコード時点(レイアウト後、フォントの
// `embed_font`/`embed_font_streaming_chunks`と同じタイミング)まで遅延する」
// 方式に変更した。理由: box tree構築(`layout::box_tree`)の時点でRefを
// 払い出してPDFへ書き出そうとすると、box tree構築が`Sink`への書き込み権限を
// 持つ必要が生じ、既存の「box tree構築は純粋にDOM+スタイルから決まる」
// という設計(バッチ/ストリーミング両モードで共有)を壊してしまう。
// `Rc<PreparedImage>`を文書内キャッシュ(このモジュールの
// `ImageAssetCache`)で共有する方式なら、フェッチ・デコードは同じく
// srcごとに1回で済み(要素数ではなく異なる画像の種類数にメモリが比例する
// という0014の核心的な要件は満たされる)、かつRef割当・書き出しは
// フォントと同じ既存のタイミング(レイアウト確定後)に置ける。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use pdf_writer::{Chunk, Ref};

use crate::img::{DocumentImageCache, ImageFetcher};

use super::document::RefAllocator;

/// [`ImageAssetCache`]1件分の結果(成功時のデコード済み画像、または失敗理由)。
type CachedDecodedImage = Result<Rc<PreparedImage>, Rc<str>>;

/// `<img>`のフェッチ→デコードまでを文書内でメモ化するキャッシュ。
///
/// `img::DocumentImageCache`(生バイト列のメモ化、T47)の上に、デコード結果
/// (`PreparedImage`)のメモ化をもう1段重ねる。同じ`src`が同一文書内で
/// 何度参照されても、フェッチ・デコードいずれも初回の1回で済む。
pub struct ImageAssetCache {
    fetcher: ImageFetcher,
    fetch_cache: DocumentImageCache,
    decoded: RefCell<HashMap<String, CachedDecodedImage>>,
}

impl ImageAssetCache {
    pub fn new(base_dir: PathBuf, allow_remote: bool) -> Self {
        Self::with_base_href(base_dir, allow_remote, None)
    }

    /// `<base href>`(相対参照の基準、[0040](
    /// ../../../docs/decisions/0040-base-href-design.md))を指定して構築する。
    pub fn with_base_href(
        base_dir: PathBuf,
        allow_remote: bool,
        base_href: Option<String>,
    ) -> Self {
        Self {
            fetcher: ImageFetcher::new(base_dir, allow_remote).with_base_href(base_href),
            fetch_cache: DocumentImageCache::new(),
            decoded: RefCell::new(HashMap::new()),
        }
    }

    /// `raw_src`(`<img src>`属性の生の値)に対応するデコード済み画像を返す。
    pub fn get_or_decode(&self, raw_src: &str) -> CachedDecodedImage {
        if let Some(cached) = self.decoded.borrow().get(raw_src) {
            return cached.clone();
        }

        let result = self
            .fetch_cache
            .get_or_fetch(&self.fetcher, raw_src)
            .and_then(|bytes| {
                decode_image(&bytes)
                    .map(Rc::new)
                    .map_err(|e| Rc::from(e.to_string()))
            });

        self.decoded
            .borrow_mut()
            .insert(raw_src.to_string(), result.clone());
        result
    }
}

/// 1枚の画像([`PreparedImage`])をPDFへ書き出すために割り当てた`Ref`。
#[derive(Debug, Clone, Copy)]
pub struct ImageIds {
    pub color: Ref,
    /// `image.alpha`が`Some`の場合のみ`Some`になる。
    pub alpha: Option<Ref>,
}

/// `image`に対応する`Ref`を、`image_ids`に無ければ新規に払い出す
/// (登録するだけで、XObjectの書き出しは呼び出し側が
/// [`embed_image`]/[`embed_image_streaming_chunks`]で行う)。
///
/// `image_ids`は`Rc::as_ptr`(デコード結果の同一性、`ImageAssetCache`が
/// 同じ`src`に対して同じ`Rc`を返すことを前提とする)をキーにした文書全体で
/// 共有するマップ。既に登録済みならそのRefを返すだけで新規に払い出さない
/// (=同じ画像はPDFへ2回書き出さない)。
pub fn ids_for_image(
    alloc: &mut RefAllocator,
    image_ids: &mut HashMap<usize, ImageIds>,
    image: &Rc<PreparedImage>,
) -> (ImageIds, bool) {
    let key = Rc::as_ptr(image) as usize;
    if let Some(&ids) = image_ids.get(&key) {
        return (ids, false);
    }
    let ids = ImageIds {
        color: alloc.next(),
        alpha: image.alpha.as_ref().map(|_| alloc.next()),
    };
    image_ids.insert(key, ids);
    (ids, true)
}

/// バッチモード向け: `image`のXObjectを`pdf`(`DerefMut<Target = Chunk>`)へ
/// 直接書き込む。
pub fn embed_image(
    pdf: &mut impl std::ops::DerefMut<Target = Chunk>,
    image: &PreparedImage,
    ids: &ImageIds,
    grayscale: bool,
) {
    let color = if grayscale {
        to_grayscale_plane(&image.color).0
    } else {
        image.color.clone()
    };
    let image = &PreparedImage {
        color,
        ..image.clone()
    };
    if let (Some(alpha), Some(alpha_id)) = (&image.alpha, ids.alpha) {
        write_plane(pdf, alpha_id, image.width, image.height, alpha, None);
    }
    write_plane(
        pdf,
        ids.color,
        image.width,
        image.height,
        &image.color,
        ids.alpha,
    );
}

/// [`embed_image`]のストリーミング版。`(Ref, Chunk)`の列を返し、呼び出し側が
/// `Sink`へ都度書き出す(フォントの`embed_font_streaming_chunks`と同じ形)。
pub fn embed_image_streaming_chunks(
    image: &PreparedImage,
    ids: &ImageIds,
    grayscale: bool,
) -> Vec<(Ref, Chunk)> {
    let color = if grayscale {
        let (plane, converted) = to_grayscale_plane(&image.color);
        if !converted {
            eprintln!(
                "警告: この画像はグレースケール化できません(JPEG/CMYKはデコーダを持たないため、[0057]決定4)"
            );
        }
        plane
    } else {
        image.color.clone()
    };
    let image = &PreparedImage {
        color,
        ..image.clone()
    };
    let mut chunks = Vec::with_capacity(2);
    if let (Some(alpha), Some(alpha_id)) = (&image.alpha, ids.alpha) {
        let mut chunk = Chunk::new();
        write_plane(&mut chunk, alpha_id, image.width, image.height, alpha, None);
        chunks.push((alpha_id, chunk));
    }
    let mut chunk = Chunk::new();
    write_plane(
        &mut chunk,
        ids.color,
        image.width,
        image.height,
        &image.color,
        ids.alpha,
    );
    chunks.push((ids.color, chunk));
    chunks
}

/// 画像のカラープレーンをグレースケール化する([0057](
/// ../../../docs/decisions/0057-pdf-output-options-design.md)決定4)。
///
/// 変換できるのはピクセルデータを持てる`Rgb`プレーン(無圧縮または
/// `/FlateDecode`)だけ。JPEGパススルー(`/DCTDecode`)とCMYKは
/// **デコーダを持たないため変換できず、そのまま返す**(既知の限界)。
/// 変換しなかった場合に`false`を返すので、呼び出し側が警告を出せる。
pub fn to_grayscale_plane(plane: &ImagePlane) -> (ImagePlane, bool) {
    if plane.color_space != PlaneColorSpace::Rgb || plane.bits_per_component != 8 {
        // Grayは変換不要、CmykとDctDecodeは変換できない。
        let converted = plane.color_space == PlaneColorSpace::Gray;
        return (plane.clone(), converted);
    }

    let raw = match plane.filter {
        Filter::FlateDecode => match inflate(&plane.data) {
            Some(bytes) => bytes,
            None => return (plane.clone(), false),
        },
        Filter::DctDecode => return (plane.clone(), false),
        _ => plane.data.clone(),
    };

    let mut gray = Vec::with_capacity(raw.len() / 3);
    for px in raw.chunks_exact(3) {
        let y = 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
        gray.push(y.round().clamp(0.0, 255.0) as u8);
    }

    let (data, filter) = match plane.filter {
        Filter::FlateDecode => (deflate(&gray), Filter::FlateDecode),
        other => (gray, other),
    };
    (
        ImagePlane {
            data,
            filter,
            color_space: PlaneColorSpace::Gray,
            bits_per_component: 8,
        },
        true,
    )
}

/// zlib展開。壊れていた場合は`None`(呼び出し側で変換をあきらめる)。
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn write_plane(
    chunk: &mut Chunk,
    id: Ref,
    width: u32,
    height: u32,
    plane: &ImagePlane,
    smask: Option<Ref>,
) {
    let mut xobject = chunk.image_xobject(id, &plane.data);
    xobject.width(width as i32);
    xobject.height(height as i32);
    match plane.color_space {
        PlaneColorSpace::Gray => {
            xobject.color_space().device_gray();
        }
        PlaneColorSpace::Rgb => {
            xobject.color_space().device_rgb();
        }
        PlaneColorSpace::Cmyk => {
            xobject.color_space().device_cmyk();
        }
    }
    xobject.bits_per_component(plane.bits_per_component);
    xobject.filter(plane.filter);
    if let Some(smask_id) = smask {
        xobject.s_mask(smask_id);
    }
    xobject.finish();
}

/// PDFの`/Resources /XObject`辞書に登録するリソース名。`Ref`番号から機械的に
/// 導出することで、ページごとの採番管理を別途持たずに済ませる。
pub fn image_resource_name(color_ref: Ref) -> String {
    format!("Im{}", color_ref.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    const JPEG_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient.jpg"
    );
    const PNG_ALPHA_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient_alpha.png"
    );
    const PNG_OPAQUE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_opaque.png"
    );
    const PNG_GRAY_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gray.png"
    );
    const WEBP_ALPHA_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_gradient_alpha.webp"
    );
    const WEBP_OPAQUE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images/spike_opaque.webp"
    );

    fn inflate(data: &[u8]) -> Vec<u8> {
        let mut decoder = ZlibDecoder::new(data);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn jpeg_is_embedded_as_a_dctdecode_passthrough() {
        let bytes = std::fs::read(JPEG_PATH).unwrap();
        let original_len = bytes.len();
        let prepared = decode_image(&bytes).expect("jpeg decode should succeed");

        assert_eq!(prepared.width, 32);
        assert_eq!(prepared.height, 24);
        assert_eq!(prepared.color.filter, Filter::DctDecode);
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Rgb);
        assert!(prepared.alpha.is_none(), "JPEG has no alpha channel");
        assert_eq!(
            prepared.color.data.len(),
            original_len,
            "passthrough must not re-encode the JPEG bytes"
        );
        assert_eq!(prepared.color.data, bytes);
    }

    #[test]
    fn png_with_alpha_splits_color_and_smask() {
        let bytes = std::fs::read(PNG_ALPHA_PATH).unwrap();
        let prepared = decode_image(&bytes).expect("png decode should succeed");

        assert_eq!(prepared.width, 16);
        assert_eq!(prepared.height, 16);
        assert_eq!(prepared.color.filter, Filter::FlateDecode);
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Rgb);

        let alpha = prepared.alpha.expect("expected an alpha plane");
        assert_eq!(alpha.color_space, PlaneColorSpace::Gray);

        let color_bytes = inflate(&prepared.color.data);
        assert_eq!(color_bytes.len(), 16 * 16 * 3);
        let alpha_bytes = inflate(&alpha.data);
        assert_eq!(alpha_bytes.len(), 16 * 16);

        // 左半分(x<8)は不透明(255)、右半分は半透明(80)で生成したフィクスチャ。
        assert_eq!(alpha_bytes[0], 255, "left half should be opaque");
        assert_eq!(alpha_bytes[8], 80, "right half should be semi-transparent");
    }

    #[test]
    fn opaque_png_has_no_alpha_plane() {
        let bytes = std::fs::read(PNG_OPAQUE_PATH).unwrap();
        let prepared = decode_image(&bytes).expect("png decode should succeed");

        assert!(prepared.alpha.is_none());
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Rgb);
        let color_bytes = inflate(&prepared.color.data);
        assert_eq!(
            color_bytes.len(),
            (prepared.width * prepared.height * 3) as usize
        );
    }

    #[test]
    fn grayscale_png_stays_devicegray_without_tripling_bytes() {
        let bytes = std::fs::read(PNG_GRAY_PATH).unwrap();
        let prepared = decode_image(&bytes).expect("png decode should succeed");

        assert!(prepared.alpha.is_none());
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Gray);
        let color_bytes = inflate(&prepared.color.data);
        assert_eq!(
            color_bytes.len(),
            (prepared.width * prepared.height) as usize,
            "grayscale should stay 1 byte/pixel, not be expanded to RGB"
        );
    }

    #[test]
    fn webp_with_alpha_splits_color_and_smask() {
        let bytes = std::fs::read(WEBP_ALPHA_PATH).unwrap();
        let prepared = decode_image(&bytes).expect("webp decode should succeed");

        assert_eq!(prepared.width, 16);
        assert_eq!(prepared.height, 16);
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Rgb);
        let alpha = prepared.alpha.expect("expected an alpha plane");

        let alpha_bytes = inflate(&alpha.data);
        assert_eq!(alpha_bytes[0], 255, "left half should be opaque");
        assert_eq!(alpha_bytes[8], 80, "right half should be semi-transparent");
    }

    #[test]
    fn opaque_webp_has_no_alpha_plane() {
        let bytes = std::fs::read(WEBP_OPAQUE_PATH).unwrap();
        let prepared = decode_image(&bytes).expect("webp decode should succeed");

        assert!(prepared.alpha.is_none());
        assert_eq!(prepared.color.color_space, PlaneColorSpace::Rgb);
    }

    #[test]
    fn unrecognized_bytes_are_rejected() {
        let result = decode_image(b"not an image");
        assert!(result.is_err());
    }

    #[test]
    fn truncated_jpeg_header_is_rejected() {
        let result = decode_image(&[0xFF, 0xD8, 0xFF]);
        assert!(result.is_err());
    }
}
