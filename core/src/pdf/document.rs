//! レイアウト結果(ページごとの[`LaidOutBox`]木)をPDFへエンコードする。
//!
//! M1は一括変換(ストリーミングなし)なので、文書全体を`pdf_writer::Pdf`で
//! 組み立てて最後に1回だけ[`Sink`]へ書き出す。ページ確定ごとに部分的な
//! バイト列を書き出すインクリメンタル対応は、T1のスパイクで実現可能性を
//! 確認済みだが、本実装への組み込みはマイルストーン3(ストリーミング対応)で行う。
//!
//! エンコードは2パスで行う: (1) 全ページを走査し、フォントごとに実際に使われた
//! グリフを集める、(2) 使用グリフだけにサブセット化したフォントを埋め込み、
//! 元グリフID→サブセット後グリフID(CID)の対応表を得てから、コンテンツ
//! ストリームを実際に書く。レイアウト時([`crate::layout::inline`])に
//! シェイピング済みの[`crate::fonts::ShapedGlyph`]をそのまま使うため、
//! テキストの再シェイピングは発生しない。
//!
//! 既知の簡略化:
//! - `border-color`は計算スタイルに保持していない(T3参照)ため、枠線の描画は
//!   行わない(`background-color`のみ描画する)
//! - ページ分割で無名化されたインライン断片(`node: None`)は初期値スタイル
//!   (黒文字・16px)で描画する
//! - 1行の中でフォントが複数使われる場合、行のベースライン位置は先頭ランの
//!   フォントのメトリクスを基準に揃える

use std::collections::HashMap;

use pdf_writer::{Content, Finish, Name, Pdf, Rect as PdfRect, Ref};

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::layout::{LaidOutBox, LaidOutContent, LineBox, Page, PageSettings, Rect};
use crate::sink::Sink;
use crate::style::ComputedStyle;

use super::font::{embed_font, FontIds, FontUsage};

/// DOM由来のレイアウト結果(ページ列)をPDFバイト列にエンコードする。
pub fn encode_pdf(
    pages: &[Page],
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut alloc = RefAllocator::default();

    let catalog_id = alloc.next();
    let pages_tree_id = alloc.next();

    let font_ids: Vec<FontIds> = (0..fonts.len())
        .map(|_| FontIds {
            font_file: alloc.next(),
            descriptor: alloc.next(),
            cid_font: alloc.next(),
            type0_font: alloc.next(),
            to_unicode: alloc.next(),
        })
        .collect();
    let font_resource_names: Vec<String> = (0..fonts.len()).map(|i| format!("F{i}")).collect();

    // Pass 1: 使用グリフを収集する(コンテンツストリームはまだ書かない)。
    let mut usages: Vec<FontUsage> = (0..fonts.len()).map(|_| FontUsage::default()).collect();
    for page in pages {
        for b in &page.boxes {
            collect_usage(b, fonts, &mut usages);
        }
    }

    // 使用グリフだけにサブセット化してフォントを埋め込み、元GID→CIDの対応表を得る。
    let remaps: Vec<HashMap<u16, u16>> = fonts
        .fonts()
        .iter()
        .zip(font_ids.iter())
        .zip(usages.iter())
        .map(|((font, &ids), usage)| embed_font(&mut pdf, font, ids, usage).into_iter().collect())
        .collect();

    // Pass 2: 実際にページのコンテンツストリームを書く。
    let mut page_ids = Vec::with_capacity(pages.len());
    for page in pages {
        let page_id = alloc.next();
        let content_id = alloc.next();
        page_ids.push(page_id);

        let mut content = Content::new();
        for b in &page.boxes {
            render_box(
                &mut content,
                b,
                styles,
                fonts,
                settings,
                &remaps,
                &font_resource_names,
            );
        }
        let content_bytes = content.finish();

        let mut p = pdf.page(page_id);
        p.parent(pages_tree_id);
        p.media_box(PdfRect::new(
            0.0,
            0.0,
            settings.size.width,
            settings.size.height,
        ));
        p.contents(content_id);
        {
            let mut resources = p.resources();
            let mut font_dict = resources.fonts();
            for (name, ids) in font_resource_names.iter().zip(font_ids.iter()) {
                font_dict.pair(Name(name.as_bytes()), ids.type0_font);
            }
        }
        p.finish();

        pdf.stream(content_id, &content_bytes);
    }

    pdf.pages(pages_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);
    pdf.catalog(catalog_id).pages(pages_tree_id);

    pdf.finish()
}

/// [`crate::layout::paginate_document`]の結果を、実際に`sink`へ書き出すところまで行う。
pub fn write_document<S: Sink>(
    pages: &[Page],
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
    mut sink: S,
) -> Result<S::Output, S::Error> {
    let bytes = encode_pdf(pages, styles, fonts, settings);
    sink.write(&bytes)?;
    sink.finish()
}

#[derive(Default)]
struct RefAllocator(i32);

impl RefAllocator {
    fn next(&mut self) -> Ref {
        self.0 += 1;
        Ref::new(self.0)
    }
}

fn collect_usage(b: &LaidOutBox, fonts: &FontCollection, usages: &mut [FontUsage]) {
    match &b.content {
        LaidOutContent::Blocks(children) => {
            for child in children {
                collect_usage(child, fonts, usages);
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines {
                for run in &line.runs {
                    let Some(font) = fonts.get(run.font_index) else {
                        continue;
                    };
                    for glyph in &run.glyphs {
                        let unicode = run.text[glyph.cluster as usize..]
                            .chars()
                            .next()
                            .unwrap_or('\u{FFFD}');
                        usages[run.font_index].record(font, glyph.glyph_id, unicode);
                    }
                }
            }
        }
    }
}

fn render_box(
    content: &mut Content,
    b: &LaidOutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
    remaps: &[HashMap<u16, u16>],
    font_resource_names: &[String],
) {
    let style = b
        .node
        .and_then(|n| styles.get(&n))
        .cloned()
        .unwrap_or_default();

    if style.background_color.alpha > 0.0 {
        render_background(
            content,
            b.layout.border_box(),
            style.background_color,
            settings,
        );
    }

    match &b.content {
        LaidOutContent::Blocks(children) => {
            for child in children {
                render_box(
                    content,
                    child,
                    styles,
                    fonts,
                    settings,
                    remaps,
                    font_resource_names,
                );
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines {
                render_line(
                    content,
                    line,
                    &style,
                    fonts,
                    settings,
                    remaps,
                    font_resource_names,
                );
            }
        }
    }
}

fn render_background(
    content: &mut Content,
    border_box: Rect,
    color: crate::style::RgbaColor,
    settings: &PageSettings,
) {
    let x = settings.margin.left + border_box.x;
    let y = to_pdf_y(settings, border_box.y + border_box.height);
    content.set_fill_rgb(
        color.red as f32 / 255.0,
        color.green as f32 / 255.0,
        color.blue as f32 / 255.0,
    );
    content.rect(x, y, border_box.width, border_box.height);
    content.fill_nonzero();
}

fn render_line(
    content: &mut Content,
    line: &LineBox,
    style: &ComputedStyle,
    fonts: &FontCollection,
    settings: &PageSettings,
    remaps: &[HashMap<u16, u16>],
    font_resource_names: &[String],
) {
    let Some(first_run) = line.runs.first() else {
        return;
    };
    let font_size = style.font_size.0;

    // 行内で複数フォントが混在していても、ベースラインは先頭ランのフォントの
    // メトリクスを基準に統一する。
    let baseline_font = fonts.get(first_run.font_index);
    let baseline_offset_px = baseline_font
        .map(|f| baseline_offset(f, font_size, line.rect.height))
        .unwrap_or(font_size);
    let baseline_y = to_pdf_y(settings, line.rect.y + baseline_offset_px);
    let color = style.color;

    content.begin_text();
    content.set_fill_rgb(
        color.red as f32 / 255.0,
        color.green as f32 / 255.0,
        color.blue as f32 / 255.0,
    );

    for run in &line.runs {
        if run.glyphs.is_empty() {
            continue;
        }
        let Some(remap) = remaps.get(run.font_index) else {
            continue;
        };
        let Some(resource_name) = font_resource_names.get(run.font_index) else {
            continue;
        };

        let mut glyph_bytes = Vec::with_capacity(run.glyphs.len() * 2);
        for glyph in &run.glyphs {
            let cid = remap.get(&glyph.glyph_id).copied().unwrap_or(0);
            glyph_bytes.extend_from_slice(&cid.to_be_bytes());
        }

        let x = settings.margin.left + line.rect.x + run.x_offset;
        content.set_font(Name(resource_name.as_bytes()), font_size);
        content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, baseline_y]);
        content.show(pdf_writer::Str(&glyph_bytes));
    }

    content.end_text();
}

/// フォントのアセント/ディセントから、行ボックス上端からベースラインまでの
/// 距離を求める(フォントのem矩形を行ボックス内で上下中央に配置する)。
fn baseline_offset(font: &crate::fonts::Font, font_size: f32, line_height: f32) -> f32 {
    let units_per_em = font.units_per_em() as f32;
    let ascent = font.ascender() as f32 / units_per_em * font_size;
    let descent = -(font.descender() as f32) / units_per_em * font_size;
    let half_leading = (line_height - (ascent + descent)) / 2.0;
    ascent + half_leading
}

/// ページコンテンツ領域上端からの距離(CSSのY、下向き正)を、PDFのユーザー空間の
/// Y座標(ページ物理下端からの距離、上向き正)に変換する。
fn to_pdf_y(settings: &PageSettings, y_from_content_top: f32) -> f32 {
    settings.size.height - settings.margin.top - y_from_content_top
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Font;
    use crate::html;
    use crate::layout::paginate_document;
    use crate::sink::MemorySink;
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const DEJAVU_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
    const CJK_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fonts/NotoSansCJK-Regular.ttc"
    );

    fn test_fonts() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).expect("should load bundled test font")
        ])
    }

    fn test_fonts_with_cjk() -> FontCollection {
        FontCollection::new(vec![
            Font::load(DEJAVU_PATH).expect("should load bundled DejaVu test font"),
            Font::load_indexed(CJK_PATH, 0).expect("should load bundled CJK test font"),
        ])
    }

    fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn encodes_a_valid_pdf_with_embedded_font() {
        let dom = html::parse(b"<p>Hello, world!</p>");
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &fonts, &settings);

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"%%EOF") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /Type0") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0);
        assert!(count_occurrences(&bytes, b"/Identity-H") > 0);
        assert!(count_occurrences(&bytes, b"/FontFile2") > 0);
        assert!(
            count_occurrences(&bytes, b"/Type /CMap") > 0,
            "ToUnicode CMap should be embedded"
        );
    }

    #[test]
    fn subsetting_keeps_embedded_font_small() {
        // CJKフォント(元は約19MB)を、短いテキストだけ使ってPDFに埋め込む。
        // サブセット化が効いていれば、出力PDF全体が元フォントよりずっと小さいはず。
        let dom = html::parse("<p>日本語のテスト</p>".as_bytes());
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts_with_cjk();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &fonts, &settings);

        let cjk_font_size = std::fs::metadata(CJK_PATH).unwrap().len() as usize;
        assert!(
            bytes.len() < cjk_font_size / 10,
            "subsetted output ({} bytes) should be far smaller than the original CJK font ({} bytes)",
            bytes.len(),
            cjk_font_size
        );
    }

    #[test]
    fn multi_page_document_produces_one_media_box_per_page() {
        let mut html_src = String::from("<div>");
        for i in 0..20 {
            html_src.push_str(&format!(r#"<p class="item">item {i}</p>"#));
        }
        html_src.push_str("</div>");
        let dom = html::parse(html_src.as_bytes());

        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(".item { height: 100px; margin: 0; }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        assert!(
            pages.len() > 1,
            "expected pagination to produce multiple pages"
        );

        let bytes = encode_pdf(&pages, &styles, &fonts, &settings);
        assert_eq!(count_occurrences(&bytes, b"/MediaBox"), pages.len());
    }

    #[test]
    fn background_color_adds_fill_drawing_to_content_stream() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom_with_bg = html::parse(br#"<div class="box">x</div>"#);
        let author_with_bg = parse_stylesheet(".box { background-color: rgb(10, 20, 30); }");
        let styles_with = compute_styles(&dom_with_bg, &ua, &author_with_bg);
        let pages_with = paginate_document(&dom_with_bg, &styles_with, &fonts, &settings);
        let bytes_with = encode_pdf(&pages_with, &styles_with, &fonts, &settings);

        let dom_without_bg = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without_bg, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without_bg, &styles_without, &fonts, &settings);
        let bytes_without = encode_pdf(&pages_without, &styles_without, &fonts, &settings);

        assert!(
            bytes_with.len() > bytes_without.len(),
            "background-color should add extra drawing operators to the content stream"
        );
    }

    #[test]
    fn mixed_script_document_embeds_both_fonts() {
        let dom = html::parse("<p>Invoice 請求書</p>".as_bytes());
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts_with_cjk();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &fonts, &settings);

        // 2つのフォント(DejaVu Sans, Noto Sans CJK JP)がそれぞれ埋め込まれているはず。
        assert_eq!(count_occurrences(&bytes, b"/FontFile2"), 2);
        assert_eq!(count_occurrences(&bytes, b"/Subtype /Type0"), 2);
    }

    #[test]
    fn write_document_writes_pdf_bytes_to_sink() {
        let dom = html::parse(b"<p>hi</p>");
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();
        let pages = paginate_document(&dom, &styles, &fonts, &settings);

        let bytes = write_document(&pages, &styles, &fonts, &settings, MemorySink::new()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
