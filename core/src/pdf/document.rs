//! レイアウト結果(ページごとの[`LaidOutBox`]木)をPDFへエンコードする。
//!
//! M1は一括変換(ストリーミングなし)なので、文書全体を`pdf_writer::Pdf`で
//! 組み立てて最後に1回だけ[`Sink`]へ書き出す。ページ確定ごとに部分的な
//! バイト列を書き出すインクリメンタル対応は、T1のスパイクで実現可能性を
//! 確認済みだが、本実装への組み込みはマイルストーン3(ストリーミング対応)で行う。
//!
//! 既知の簡略化:
//! - `border-color`は計算スタイルに保持していない(T3参照)ため、枠線の描画は
//!   行わない(`background-color`のみ描画する)
//! - ページ分割で無名化されたインライン断片(`node: None`)は初期値スタイル
//!   (黒文字・16px)で描画する

use std::collections::{BTreeMap, HashMap};

use pdf_writer::{Content, Finish, Name, Pdf, Rect as PdfRect, Ref, Str};

use crate::fonts::{shape_text, Font};
use crate::html::NodeId;
use crate::layout::{LaidOutBox, LaidOutContent, LineBox, Page, PageSettings, Rect};
use crate::sink::Sink;
use crate::style::ComputedStyle;

use super::font::{embed_font, record_glyph_width, FontIds};

/// DOM由来のレイアウト結果(ページ列)をPDFバイト列にエンコードする。
pub fn encode_pdf(
    pages: &[Page],
    styles: &HashMap<NodeId, ComputedStyle>,
    font: &Font,
    settings: &PageSettings,
) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut alloc = RefAllocator::default();

    let catalog_id = alloc.next();
    let pages_tree_id = alloc.next();
    let font_ids = FontIds {
        font_file: alloc.next(),
        descriptor: alloc.next(),
        cid_font: alloc.next(),
        type0_font: alloc.next(),
    };

    let mut used_glyphs = BTreeMap::new();
    let mut page_ids = Vec::with_capacity(pages.len());

    for page in pages {
        let page_id = alloc.next();
        let content_id = alloc.next();
        page_ids.push(page_id);

        let mut content = Content::new();
        for b in &page.boxes {
            render_box(&mut content, b, styles, font, settings, &mut used_glyphs);
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
        p.resources().fonts().pair(Name(b"F1"), font_ids.type0_font);
        p.finish();

        pdf.stream(content_id, &content_bytes);
    }

    pdf.pages(pages_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);
    pdf.catalog(catalog_id).pages(pages_tree_id);

    embed_font(&mut pdf, font, font_ids, &used_glyphs);

    pdf.finish()
}

/// [`crate::layout::paginate_document`]の結果を、実際に`sink`へ書き出すところまで行う。
pub fn write_document<S: Sink>(
    pages: &[Page],
    styles: &HashMap<NodeId, ComputedStyle>,
    font: &Font,
    settings: &PageSettings,
    mut sink: S,
) -> Result<S::Output, S::Error> {
    let bytes = encode_pdf(pages, styles, font, settings);
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

fn render_box(
    content: &mut Content,
    b: &LaidOutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    font: &Font,
    settings: &PageSettings,
    used_glyphs: &mut BTreeMap<u16, f32>,
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
                render_box(content, child, styles, font, settings, used_glyphs);
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines {
                render_line(content, line, &style, font, settings, used_glyphs);
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
    font: &Font,
    settings: &PageSettings,
    used_glyphs: &mut BTreeMap<u16, f32>,
) {
    if line.text.trim().is_empty() {
        return;
    }

    let font_size = style.font_size.0;
    // T6のレイアウト時にも同じ入力でシェイピング済みだが、グリフID列は
    // LineBoxに保持していないため再計算する(決定論的なので結果は一致する)。
    let shaped = shape_text(font, &line.text, font_size);
    if shaped.glyphs.is_empty() {
        return;
    }

    for g in &shaped.glyphs {
        record_glyph_width(font, g.glyph_id, used_glyphs);
    }

    let mut glyph_bytes = Vec::with_capacity(shaped.glyphs.len() * 2);
    for g in &shaped.glyphs {
        glyph_bytes.extend_from_slice(&g.glyph_id.to_be_bytes());
    }

    let baseline_y = to_pdf_y(
        settings,
        line.rect.y + baseline_offset(font, font_size, line.rect.height),
    );
    let x = settings.margin.left + line.rect.x;
    let color = style.color;

    content.begin_text();
    content.set_fill_rgb(
        color.red as f32 / 255.0,
        color.green as f32 / 255.0,
        color.blue as f32 / 255.0,
    );
    content.set_font(Name(b"F1"), font_size);
    content.next_line(x, baseline_y);
    content.show(Str(&glyph_bytes));
    content.end_text();
}

/// フォントのアセント/ディセントから、行ボックス上端からベースラインまでの
/// 距離を求める(フォントのem矩形を行ボックス内で上下中央に配置する)。
fn baseline_offset(font: &Font, font_size: f32, line_height: f32) -> f32 {
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
    use crate::html;
    use crate::layout::paginate_document;
    use crate::sink::MemorySink;
    use crate::style::{compute_styles, parse_stylesheet, user_agent_stylesheet, Stylesheet};

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    fn test_font() -> Font {
        Font::load(TEST_FONT_PATH).expect("should load bundled test font")
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
        let font = test_font();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &font, &settings);
        let bytes = encode_pdf(&pages, &styles, &font, &settings);

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(count_occurrences(&bytes, b"%%EOF") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /Type0") > 0);
        assert!(count_occurrences(&bytes, b"/Subtype /CIDFontType2") > 0);
        assert!(count_occurrences(&bytes, b"/Identity-H") > 0);
        assert!(count_occurrences(&bytes, b"/FontFile2") > 0);
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
        let font = test_font();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &font, &settings);
        assert!(
            pages.len() > 1,
            "expected pagination to produce multiple pages"
        );

        let bytes = encode_pdf(&pages, &styles, &font, &settings);
        assert_eq!(count_occurrences(&bytes, b"/MediaBox"), pages.len());
    }

    #[test]
    fn background_color_adds_fill_drawing_to_content_stream() {
        let ua = user_agent_stylesheet();
        let font = test_font();
        let settings = PageSettings::default();

        let dom_with_bg = html::parse(br#"<div class="box">x</div>"#);
        let author_with_bg = parse_stylesheet(".box { background-color: rgb(10, 20, 30); }");
        let styles_with = compute_styles(&dom_with_bg, &ua, &author_with_bg);
        let pages_with = paginate_document(&dom_with_bg, &styles_with, &font, &settings);
        let bytes_with = encode_pdf(&pages_with, &styles_with, &font, &settings);

        let dom_without_bg = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without_bg, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without_bg, &styles_without, &font, &settings);
        let bytes_without = encode_pdf(&pages_without, &styles_without, &font, &settings);

        assert!(
            bytes_with.len() > bytes_without.len(),
            "background-color should add extra drawing operators to the content stream"
        );
    }

    #[test]
    fn write_document_writes_pdf_bytes_to_sink() {
        let dom = html::parse(b"<p>hi</p>");
        let ua = user_agent_stylesheet();
        let author = Stylesheet::default();
        let styles = compute_styles(&dom, &ua, &author);
        let font = test_font();
        let settings = PageSettings::default();
        let pages = paginate_document(&dom, &styles, &font, &settings);

        let bytes = write_document(&pages, &styles, &font, &settings, MemorySink::new()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
