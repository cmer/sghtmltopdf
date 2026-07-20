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
//! テキストの色・太字・イタリックは[`crate::layout::inline::TextRun`]に
//! レイアウト時点で焼き込み済み(`<b>`/`<span style="...">`等のインライン要素
//! ごとに異なりうる)なので、ページ分割で無名化されたインライン断片
//! (`node: None`)であっても正しい見た目で描画される。
//!
//! 枠線は`border-style`が`none`でなく、かつ幅が0より大きい辺のみ描画する。
//! `solid`/`double`は、border-box外周から内周までを辺ごとの四角形(太さが
//! 不揃いなら台形)として塗りつぶす。隣接する2辺は共有する頂点(外側の角・
//! 内側の角)から独立に頂点を計算するため、太さ・色が異なっていても角が
//! 斜めにミトー結合される(ピクチャーフレームと同じ要領)。`dashed`/`dotted`
//! はダッシュパターンをストロークで表現する都合上、太さの中心線を
//! ストロークする従来方式のまま(ミトー結合はしない)。
//! `border-radius`が指定されておらず、かつ4辺すべての太さ・スタイル・色が
//! 同一の場合は角丸のベジェ曲線パスでまとめてストロークし、それ以外
//! (角丸なし、または4辺が不揃い)は上記の辺ごとの描画にフォールバックする。
//!
//! ページ分割で断片化したボックス([`crate::layout::FragmentPosition`]参照)は、
//! 継続中の辺(分割位置に接する辺)に`border-radius`を適用しない
//! (レイアウト側で`Layout::fragment`として渡された情報を見て角丸を抑制する)。
//!
//! 既知の簡略化:
//! - 太字・イタリックは対応する字形を持つフォントファイルを別途要求せず、
//!   通常字形に対して塗り+縁取り(疑似太字)・テキスト行列のせん断(疑似
//!   イタリック)で代用する
//! - 1行の中で複数フォント・複数フォントサイズが混在する場合、行のベースライン
//!   位置は先頭ランのフォント・サイズのメトリクスを基準に揃える
//! - `border-radius`が指定されていても4辺の太さ・スタイル・色が不揃いな場合は
//!   角丸を諦め、直線4辺のストロークにフォールバックする(角ごとの複雑な
//!   ブレンド処理は非対応)
//! - `border-style`の`groove`/`ridge`/`inset`/`outset`(2階調の疑似立体陰影)は
//!   非対応。請求書・帳票用途での実用性に対して実装コストが見合わないため

use std::collections::HashMap;
use std::rc::Rc;

use pdf_writer::types::{LineCapStyle, TextRenderingMode};
use pdf_writer::{Content, Finish, Name, Pdf, Rect as PdfRect, Ref, TextStr};

use crate::fonts::FontCollection;
use crate::html::NodeId;
use crate::layout::{
    resolve_border, EdgeSizes, FragmentPosition, LaidOutBox, LaidOutContent, Layout, LineBox, Page,
    PageSettings, Rect,
};
use crate::sink::Sink;
use crate::style::{BorderCollapse, BorderStyle, ComputedStyle, EmptyCells, Length, RgbaColor};

use super::font::{deflate, embed_font, FontIds, FontUsage};
use super::img::{embed_image, ids_for_image, image_resource_name, ImageIds, PreparedImage};

/// DOM由来のレイアウト結果(ページ列)をPDFバイト列にエンコードする。
pub fn encode_pdf(
    pages: &[Page],
    styles: &HashMap<NodeId, ComputedStyle>,
    background_images: &HashMap<NodeId, Rc<PreparedImage>>,
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
            // `encode_pdf`は`/CIDToGIDMap /Identity`(embed_font)を使うため
            // 参照しないが、`FontIds`を`embed_font_streaming_chunks`と共通の
            // 型に保つため確保だけしておく。
            cid_to_gid_map: alloc.next(),
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

    // Pass 2: 実際にページのコンテンツストリームを書く。画像XObjectは、
    // フォントと違ってページ間で使い回すための事前サブセット化情報が
    // 不要なため、ページごとに「初出なら書き出す」形で済ませる
    // ([0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)参照)。
    let mut image_ids: HashMap<usize, ImageIds> = HashMap::new();
    let mut page_ids = Vec::with_capacity(pages.len());
    for page in pages {
        let page_id = alloc.next();
        let content_id = alloc.next();
        page_ids.push(page_id);

        let mut used_images = Vec::new();
        for b in &page.boxes {
            collect_image_uses(b, background_images, &mut used_images);
        }
        let mut page_image_refs = Vec::with_capacity(used_images.len());
        for image in &used_images {
            let (ids, is_new) = ids_for_image(&mut alloc, &mut image_ids, image);
            if is_new {
                embed_image(&mut pdf, image, &ids);
            }
            page_image_refs.push(ids.color);
        }

        let mut content = Content::new();
        for b in &page.boxes {
            render_box(
                &mut content,
                b,
                styles,
                fonts,
                settings,
                Some(&remaps),
                &font_resource_names,
                &image_ids,
                background_images,
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
            font_dict.finish();
            let mut xobject_dict = resources.x_objects();
            for color_ref in &page_image_refs {
                xobject_dict.pair(Name(image_resource_name(*color_ref).as_bytes()), *color_ref);
            }
        }
        p.finish();

        let compressed_content = deflate(&content_bytes);
        let mut content_stream = pdf.stream(content_id, &compressed_content);
        content_stream.filter(pdf_writer::Filter::FlateDecode);
        content_stream.finish();
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
    background_images: &HashMap<NodeId, Rc<PreparedImage>>,
    fonts: &FontCollection,
    settings: &PageSettings,
    mut sink: S,
) -> Result<S::Output, S::Error> {
    let bytes = encode_pdf(pages, styles, background_images, fonts, settings);
    sink.write(&bytes)?;
    sink.finish()
}

#[derive(Default)]
pub(super) struct RefAllocator(i32);

impl RefAllocator {
    pub(super) fn next(&mut self) -> Ref {
        self.0 += 1;
        Ref::new(self.0)
    }
}

pub(super) fn collect_usage(b: &LaidOutBox, fonts: &FontCollection, usages: &mut [FontUsage]) {
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
        LaidOutContent::Table(table) => {
            if let Some(caption) = &table.caption {
                collect_usage(caption, fonts, usages);
            }
            for row in &table.rows {
                for cell in &row.cells {
                    collect_usage(cell, fonts, usages);
                }
            }
        }
        LaidOutContent::Image(_) => {}
    }
}

/// ページ(群)を再帰的に走査し、実際に使われている画像(`<img>`本体と
/// `background-image`の両方)を`Rc`のポインタアイデンティティで重複排除して
/// 集める。フォントの`collect_usage`と同じ「使用状況を先に集めてから
/// Refを払い出す」構造。`background_images`は[0017](../../../docs/decisions/0017-background-image-design.md)
/// 決定2の`NodeId → Rc<PreparedImage>`側マップ。
pub(super) fn collect_image_uses(
    b: &LaidOutBox,
    background_images: &HashMap<NodeId, Rc<PreparedImage>>,
    out: &mut Vec<Rc<PreparedImage>>,
) {
    if let Some(image) = b.node.and_then(|n| background_images.get(&n)) {
        push_unique_image(out, image);
    }

    match &b.content {
        LaidOutContent::Blocks(children) => {
            for child in children {
                collect_image_uses(child, background_images, out);
            }
        }
        LaidOutContent::Table(table) => {
            if let Some(caption) = &table.caption {
                collect_image_uses(caption, background_images, out);
            }
            for row in &table.rows {
                for cell in &row.cells {
                    collect_image_uses(cell, background_images, out);
                }
            }
        }
        LaidOutContent::Image(Some(image)) => push_unique_image(out, image),
        LaidOutContent::Image(None) | LaidOutContent::Inline(_) => {}
    }
}

fn push_unique_image(out: &mut Vec<Rc<PreparedImage>>, image: &Rc<PreparedImage>) {
    if !out.iter().any(|existing| Rc::ptr_eq(existing, image)) {
        out.push(image.clone());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_box(
    content: &mut Content,
    b: &LaidOutBox,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
    remaps: Option<&[HashMap<u16, u16>]>,
    font_resource_names: &[String],
    image_ids: &HashMap<usize, ImageIds>,
    background_images: &HashMap<NodeId, Rc<PreparedImage>>,
) {
    let style = b
        .node
        .and_then(|n| styles.get(&n))
        .cloned()
        .unwrap_or_default();
    render_box_with_style(
        content,
        b,
        &style,
        styles,
        fonts,
        settings,
        remaps,
        font_resource_names,
        image_ids,
        background_images,
    );
}

/// [`render_box`]の本体。通常は`b.node`から引いた素の`style`で描画するが、
/// `border-collapse: collapse`時のセル(隣接セルと統合した枠線を使う必要が
/// ある)のように、呼び出し側が上書きしたスタイルで描画したい場合のために
/// `style`を引数として分離してある。
#[allow(clippy::too_many_arguments)]
fn render_box_with_style(
    content: &mut Content,
    b: &LaidOutBox,
    style: &ComputedStyle,
    styles: &HashMap<NodeId, ComputedStyle>,
    fonts: &FontCollection,
    settings: &PageSettings,
    remaps: Option<&[HashMap<u16, u16>]>,
    font_resource_names: &[String],
    image_ids: &HashMap<usize, ImageIds>,
    background_images: &HashMap<NodeId, Rc<PreparedImage>>,
) {
    // `background-image`は`<img>`と異なりboxの中身ではなく装飾なので、
    // `b.node`から側マップを引いて`Ref`を解決する([0017]決定2)。
    let background_image_ref = b
        .node
        .and_then(|n| background_images.get(&n))
        .and_then(|image| image_ids.get(&(Rc::as_ptr(image) as usize)))
        .map(|ids| ids.color);

    render_box_decoration(content, &b.layout, style, settings, background_image_ref);

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
                    image_ids,
                    background_images,
                );
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines {
                render_line(content, line, fonts, settings, remaps, font_resource_names);
            }
        }
        LaidOutContent::Image(image) => {
            if let Some(image) = image {
                if let Some(&ids) = image_ids.get(&(Rc::as_ptr(image) as usize)) {
                    render_image(content, b.layout.content, settings, ids.color);
                }
            }
        }
        LaidOutContent::Table(table) => {
            if let Some(caption) = &table.caption {
                render_box(
                    content,
                    caption,
                    styles,
                    fonts,
                    settings,
                    remaps,
                    font_resource_names,
                    image_ids,
                    background_images,
                );
            }
            // `border-collapse`は`table`/`inline-table`要素にのみ適用されるため
            // テーブル自身の`style`を見るが、`empty-cells`は`table-cell`要素に
            // 適用されるプロパティ(CSS2.1 17.6.1.1)なのでセル自身の計算済み
            // スタイルを見る必要がある(セル単位の上書きに対応するため)。
            // `empty-cells: hide`は`border-collapse: separate`でのみ意味を持つ
            // (CSS仕様通り)。内容が空のセルは元々装飾以外何も描画しないため、
            // `render_box`呼び出し自体をスキップしてよい。
            let collapse = style.border_collapse == BorderCollapse::Collapse;
            // `border-collapse: collapse`時、隣接セル間の枠線を統合するために
            // 全セルのフラットな一覧が要る(隣接判定は矩形の接触で幾何的に行う、
            // [0021]決定1・2)。
            let all_cells: Vec<&LaidOutBox> = if collapse {
                table.rows.iter().flat_map(|row| &row.cells).collect()
            } else {
                Vec::new()
            };
            for row in &table.rows {
                for cell in &row.cells {
                    let cell_style = cell
                        .node
                        .and_then(|n| styles.get(&n))
                        .cloned()
                        .unwrap_or_default();
                    let hide_this_cell = !collapse
                        && cell_style.empty_cells == EmptyCells::Hide
                        && laid_content_is_empty(&cell.content);
                    if hide_this_cell {
                        continue;
                    }
                    if collapse {
                        let (resolved_style, resolved_border) =
                            resolve_collapsed_cell_style(cell, &cell_style, &all_cells, styles);
                        // 枠線の描画太さは`ComputedStyle`ではなく`layout.border`
                        // (レイアウト確定時に計算済み)を見るため
                        // ([`render_border`]参照)、統合後の太さを反映した
                        // クローンを作って描画する。
                        let mut resolved_cell = cell.clone();
                        resolved_cell.layout.border = resolved_border;
                        render_box_with_style(
                            content,
                            &resolved_cell,
                            &resolved_style,
                            styles,
                            fonts,
                            settings,
                            remaps,
                            font_resource_names,
                            image_ids,
                            background_images,
                        );
                    } else {
                        render_box(
                            content,
                            cell,
                            styles,
                            fonts,
                            settings,
                            remaps,
                            font_resource_names,
                            image_ids,
                            background_images,
                        );
                    }
                }
            }
        }
    }
}

/// セルの内容が空かどうか(テキストが空白のみ/子要素が無い)。`empty-cells:
/// hide`の判定に使う。ネストしたテーブル・置換要素(`<img>`)は常に非空扱い
/// (内容として意味を持つため)。
fn laid_content_is_empty(content: &LaidOutContent) -> bool {
    match content {
        LaidOutContent::Inline(lines) => lines
            .iter()
            .all(|line| line.runs.iter().all(|run| run.text.trim().is_empty())),
        LaidOutContent::Blocks(children) => {
            children.is_empty() || children.iter().all(|c| laid_content_is_empty(&c.content))
        }
        LaidOutContent::Table(_) | LaidOutContent::Image(_) => false,
    }
}

/// `border-collapse: collapse`時、`cell`の枠線を隣接セルと統合した
/// `ComputedStyle`と、実際に描画する枠線の太さ(`layout.border`の差し替え用
/// `EdgeSizes`)を作る(枠線以外は`cell_style`のまま)。
///
/// レイアウト自体は`border-collapse`の値に関わらずseparateモデルと同一に
/// 保っている([0021](../../../docs/decisions/0021-table-layout-design.md)決定1)ため、collapse時は
/// `h_spacing`/`v_spacing`が0になり、隣接セルの矩形は座標が一致するまで接する。
/// これを利用して、矩形の接触を幾何的に判定するだけでrowspan/colspanの
/// グリッド情報を別途持たずに隣接セルを見つけられる。
///
/// 同じ境界が両側から二重に描画されるのを防ぐため、常に「左隣が見つかれば
/// 自分の左辺は描画しない(右隣側が統合済みの枠線を右辺として描画する)」
/// という向きで統一する(上/下も同様)。cellとtable自体の境界は対象外
/// ([0021]決定2、テーブル自身の枠線はborder-box外側の帯として描画され
/// セルの矩形と重ならないため、二重描画の問題が生じない)。1つの辺に複数の
/// 隣接セルが接する場合(rowspanが絡む場合等)は、先に見つかったものを使う
/// (既知の簡略化)。
///
/// 枠線の実際の描画太さは(`ComputedStyle`ではなく)レイアウト確定時に計算
/// 済みの`layout.border`(`EdgeSizes`)を見る([`render_border`]参照)ため、
/// 統合後の`ComputedStyle`だけでなく、それに対応する`EdgeSizes`
/// (`layout::resolve_border`と同じ正規化)も併せて返し、呼び出し側で
/// `cell.layout.border`を差し替えてもらう。
fn resolve_collapsed_cell_style(
    cell: &LaidOutBox,
    cell_style: &ComputedStyle,
    all_cells: &[&LaidOutBox],
    styles: &HashMap<NodeId, ComputedStyle>,
) -> (ComputedStyle, EdgeSizes) {
    // 矩形の接触判定に許容する誤差(浮動小数点の丸め対策)。
    const EPSILON: f32 = 0.5;

    fn ranges_overlap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> bool {
        a_start < b_end - EPSILON && b_start < a_end - EPSILON
    }

    let rect = cell.layout.border_box();
    let mut resolved = cell_style.clone();

    let has_left_neighbor = all_cells.iter().any(|other| {
        let o = other.layout.border_box();
        (o.x + o.width - rect.x).abs() < EPSILON
            && ranges_overlap(rect.y, rect.y + rect.height, o.y, o.y + o.height)
    });
    if has_left_neighbor {
        resolved.border_left_style = BorderStyle::None;
        resolved.border_left_width = Length(0.0);
    }

    let has_top_neighbor = all_cells.iter().any(|other| {
        let o = other.layout.border_box();
        (o.y + o.height - rect.y).abs() < EPSILON
            && ranges_overlap(rect.x, rect.x + rect.width, o.x, o.x + o.width)
    });
    if has_top_neighbor {
        resolved.border_top_style = BorderStyle::None;
        resolved.border_top_width = Length(0.0);
    }

    if let Some(right_neighbor) = all_cells.iter().find(|other| {
        let o = other.layout.border_box();
        (o.x - (rect.x + rect.width)).abs() < EPSILON
            && ranges_overlap(rect.y, rect.y + rect.height, o.y, o.y + o.height)
    }) {
        let neighbor_style = right_neighbor
            .node
            .and_then(|n| styles.get(&n))
            .cloned()
            .unwrap_or_default();
        let own = border_edge(
            cell_style.border_right_width.0,
            cell_style.border_right_style,
            cell_style.border_right_color,
        );
        let theirs = border_edge(
            neighbor_style.border_left_width.0,
            neighbor_style.border_left_style,
            neighbor_style.border_left_color,
        );
        let (width, style, color) = resolve_border_conflict(own, theirs);
        resolved.border_right_width = Length(width);
        resolved.border_right_style = style;
        resolved.border_right_color = color;
    }

    if let Some(bottom_neighbor) = all_cells.iter().find(|other| {
        let o = other.layout.border_box();
        (o.y - (rect.y + rect.height)).abs() < EPSILON
            && ranges_overlap(rect.x, rect.x + rect.width, o.x, o.x + o.width)
    }) {
        let neighbor_style = bottom_neighbor
            .node
            .and_then(|n| styles.get(&n))
            .cloned()
            .unwrap_or_default();
        let own = border_edge(
            cell_style.border_bottom_width.0,
            cell_style.border_bottom_style,
            cell_style.border_bottom_color,
        );
        let theirs = border_edge(
            neighbor_style.border_top_width.0,
            neighbor_style.border_top_style,
            neighbor_style.border_top_color,
        );
        let (width, style, color) = resolve_border_conflict(own, theirs);
        resolved.border_bottom_width = Length(width);
        resolved.border_bottom_style = style;
        resolved.border_bottom_color = color;
    }

    let border = resolve_border(&resolved);
    (resolved, border)
}

/// `style: none`の辺は指定幅に関わらず実効幅0として扱う
/// (`layout::resolve_border`と同じ正規化、`resolve_border_conflict`の
/// 幅比較を単純にするため)。
fn border_edge(width: f32, style: BorderStyle, color: RgbaColor) -> (f32, BorderStyle, RgbaColor) {
    if style == BorderStyle::None {
        (0.0, BorderStyle::None, color)
    } else {
        (width, style, color)
    }
}

/// CSS2.1 §17.6.2の枠線競合解決の簡略版: 幅が太い方が勝ち、幅が同じなら
/// スタイルの優先順位(強さの見た目順: double > solid > dashed > dotted >
/// none)で決める。`hidden`/`ridge`/`outset`/`groove`/`inset`は
/// [`BorderStyle`]に無いため非対応([0021]決定2)。幅・スタイルとも
/// 同着の場合は`a`を採用する(既知の簡略化、見た目には影響しない)。
fn resolve_border_conflict(
    a: (f32, BorderStyle, RgbaColor),
    b: (f32, BorderStyle, RgbaColor),
) -> (f32, BorderStyle, RgbaColor) {
    if a.0 != b.0 {
        return if a.0 > b.0 { a } else { b };
    }
    fn style_priority(s: BorderStyle) -> u8 {
        match s {
            BorderStyle::Double => 4,
            BorderStyle::Solid => 3,
            BorderStyle::Dashed => 2,
            BorderStyle::Dotted => 1,
            BorderStyle::None => 0,
        }
    }
    if style_priority(a.1) != style_priority(b.1) {
        return if style_priority(a.1) > style_priority(b.1) {
            a
        } else {
            b
        };
    }
    a
}

/// 背景・枠線を描画する。角丸(`border-radius`)が指定されていなければ従来通り
/// 直線の矩形/4辺独立ストロークで描き、指定されていれば[`render_rounded_decoration`]
/// に委譲する。`background_image_ref`はborder-boxいっぱいにストレッチ表示する
/// 背景画像のXObject Ref([0017](../../../docs/decisions/0017-background-image-design.md)
/// 決定3、`border-radius`によるクリップは非対応)。背景色→背景画像→枠線の順で
/// 描画する。
fn render_box_decoration(
    content: &mut Content,
    layout: &Layout,
    style: &ComputedStyle,
    settings: &PageSettings,
    background_image_ref: Option<Ref>,
) {
    let radii = effective_radii(layout, style);
    let has_radius = radii.0 > 0.0 || radii.1 > 0.0 || radii.2 > 0.0 || radii.3 > 0.0;

    if has_radius {
        render_rounded_decoration(
            content,
            layout,
            style,
            settings,
            radii,
            background_image_ref,
        );
        return;
    }

    if style.background_color.alpha > 0.0 {
        render_background(
            content,
            layout.border_box(),
            style.background_color,
            settings,
        );
    }
    if let Some(image_ref) = background_image_ref {
        render_image(content, layout.border_box(), settings, image_ref);
    }
    render_border(content, layout, style, settings);
}

/// スタイル上の`border-radius`を、そのボックスがページ分割された断片の
/// どの位置にあるか([`FragmentPosition`])に応じて丸める。継続中の断片
/// (`Middle`/上端なら`Last`/下端なら`First`)では、本来枠線が無い辺の角を
/// 丸めてしまわないよう、その辺に接する角の半径を0にする。
fn effective_radii(layout: &Layout, style: &ComputedStyle) -> (f32, f32, f32, f32) {
    let apply_top = matches!(
        layout.fragment,
        FragmentPosition::Whole | FragmentPosition::First
    );
    let apply_bottom = matches!(
        layout.fragment,
        FragmentPosition::Whole | FragmentPosition::Last
    );
    (
        if apply_top {
            style.border_top_left_radius.0
        } else {
            0.0
        },
        if apply_top {
            style.border_top_right_radius.0
        } else {
            0.0
        },
        if apply_bottom {
            style.border_bottom_right_radius.0
        } else {
            0.0
        },
        if apply_bottom {
            style.border_bottom_left_radius.0
        } else {
            0.0
        },
    )
}

fn render_background(
    content: &mut Content,
    border_box: Rect,
    color: RgbaColor,
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

/// `rect`いっぱいに画像XObjectを描画する。`<img>`(content box)・
/// `background-image`(border-box、[0017]決定3)いずれの呼び出し元からも
/// 使う共通ヘルパー。`resource_ref`が指すXObjectは、呼び出し元がページの
/// `/Resources/XObject`辞書へ既に登録済みであること
/// ([`image_resource_name`]と同じ命名規則でリソース名を導出する)。
fn render_image(content: &mut Content, rect: Rect, settings: &PageSettings, resource_ref: Ref) {
    let x = settings.margin.left + rect.x;
    let y = to_pdf_y(settings, rect.y + rect.height);
    let name = image_resource_name(resource_ref);
    content.save_state();
    content.transform([rect.width, 0.0, 0.0, rect.height, x, y]);
    content.x_object(Name(name.as_bytes()));
    content.restore_state();
}

/// `border-radius`が指定されている場合の背景・枠線描画。
///
/// 背景は各角の半径に従った角丸矩形として塗りつぶす。枠線は、4辺すべての
/// 太さ・スタイル・色が同一の場合のみ角丸パスをストロークする
/// (辺ごとに異なる太さ・色・スタイルと角丸の組み合わせは、角での複雑な
/// ブレンド処理が必要になるためM1では非対応。その場合は角丸を諦め、
/// 直線4辺の[`render_border`]にフォールバックする)。
#[allow(clippy::too_many_arguments)]
fn render_rounded_decoration(
    content: &mut Content,
    layout: &Layout,
    style: &ComputedStyle,
    settings: &PageSettings,
    radii: (f32, f32, f32, f32),
    background_image_ref: Option<Ref>,
) {
    let border_box = layout.border_box();
    let x0 = settings.margin.left + border_box.x;
    let x1 = x0 + border_box.width;
    let y_top = to_pdf_y(settings, border_box.y);
    let y_bottom = to_pdf_y(settings, border_box.y + border_box.height);

    if style.background_color.alpha > 0.0 {
        content.set_fill_rgb(
            style.background_color.red as f32 / 255.0,
            style.background_color.green as f32 / 255.0,
            style.background_color.blue as f32 / 255.0,
        );
        rounded_rect_path(content, x0, y_top, x1, y_bottom, radii);
        content.fill_nonzero();
    }
    // 角丸パスへのクリップは行わず、常に直線の矩形として描画する
    // (border-radiusとの組み合わせは非対応、[0017]決定3の既知の簡略化)。
    if let Some(image_ref) = background_image_ref {
        render_image(content, border_box, settings, image_ref);
    }

    if !is_uniform_border(style) {
        render_border(content, layout, style, settings);
        return;
    }

    let thickness = layout.border.top;
    if thickness <= 0.0 || style.border_top_style == BorderStyle::None {
        return;
    }

    content.set_stroke_rgb(
        style.border_top_color.red as f32 / 255.0,
        style.border_top_color.green as f32 / 255.0,
        style.border_top_color.blue as f32 / 255.0,
    );

    if style.border_top_style == BorderStyle::Double {
        // 太さを3等分し、外周から1/6・5/6の位置(それぞれの帯の中心線)に
        // 1/3幅の角丸パスを2本ストロークする(中央の1/3は空白として残る)。
        let band = thickness / 3.0;
        content.set_line_cap(LineCapStyle::ButtCap);
        content.set_dash_pattern([], 0.0);
        content.set_line_width(band);
        for offset in [band / 2.0, thickness - band / 2.0] {
            rounded_rect_path(
                content,
                x0 + offset,
                y_top - offset,
                x1 - offset,
                y_bottom + offset,
                shrink_radii(radii, offset),
            );
            content.stroke();
        }
        return;
    }

    // ストロークは太さの中心線を通るため、外周パスを半分だけ内側へ詰める
    // (半径も同じ量だけ縮める簡易近似)。
    let inset = thickness / 2.0;
    content.set_line_width(thickness);
    apply_border_style_dash(content, style.border_top_style, thickness);
    rounded_rect_path(
        content,
        x0 + inset,
        y_top - inset,
        x1 - inset,
        y_bottom + inset,
        shrink_radii(radii, inset),
    );
    content.stroke();
}

/// 4辺すべての`border-width`/`border-style`/`border-color`が一致するか。
fn is_uniform_border(style: &ComputedStyle) -> bool {
    style.border_top_width == style.border_right_width
        && style.border_top_width == style.border_bottom_width
        && style.border_top_width == style.border_left_width
        && style.border_top_style == style.border_right_style
        && style.border_top_style == style.border_bottom_style
        && style.border_top_style == style.border_left_style
        && style.border_top_color == style.border_right_color
        && style.border_top_color == style.border_bottom_color
        && style.border_top_color == style.border_left_color
}

/// 四分円をベジェ曲線で近似する際の制御点オフセット係数。
const BEZIER_KAPPA: f32 = 0.552_284_8;

/// PDF空間(Y-up、`y_top` > `y_bottom`)で角丸矩形のパスを構築して閉じる
/// (塗り/ストロークは呼び出し側が行う)。半径は`(top_left, top_right,
/// bottom_right, bottom_left)`の順(CSSの`border-radius`と同じ並び)。
fn rounded_rect_path(
    content: &mut Content,
    x0: f32,
    y_top: f32,
    x1: f32,
    y_bottom: f32,
    radii: (f32, f32, f32, f32),
) {
    let max_r = ((x1 - x0) / 2.0)
        .max(0.0)
        .min(((y_top - y_bottom) / 2.0).max(0.0));
    let (r_tl, r_tr, r_br, r_bl) = radii;
    let r_tl = r_tl.clamp(0.0, max_r);
    let r_tr = r_tr.clamp(0.0, max_r);
    let r_br = r_br.clamp(0.0, max_r);
    let r_bl = r_bl.clamp(0.0, max_r);

    content.move_to(x0 + r_tl, y_top);
    content.line_to(x1 - r_tr, y_top);
    if r_tr > 0.0 {
        let k = r_tr * BEZIER_KAPPA;
        content.cubic_to(x1 - r_tr + k, y_top, x1, y_top - r_tr + k, x1, y_top - r_tr);
    }
    content.line_to(x1, y_bottom + r_br);
    if r_br > 0.0 {
        let k = r_br * BEZIER_KAPPA;
        content.cubic_to(
            x1,
            y_bottom + r_br - k,
            x1 - r_br + k,
            y_bottom,
            x1 - r_br,
            y_bottom,
        );
    }
    content.line_to(x0 + r_bl, y_bottom);
    if r_bl > 0.0 {
        let k = r_bl * BEZIER_KAPPA;
        content.cubic_to(
            x0 + r_bl - k,
            y_bottom,
            x0,
            y_bottom + r_bl - k,
            x0,
            y_bottom + r_bl,
        );
    }
    content.line_to(x0, y_top - r_tl);
    if r_tl > 0.0 {
        let k = r_tl * BEZIER_KAPPA;
        content.cubic_to(x0, y_top - r_tl + k, x0 + r_tl - k, y_top, x0 + r_tl, y_top);
    }
    content.close_path();
}

fn shrink_radii(radii: (f32, f32, f32, f32), inset: f32) -> (f32, f32, f32, f32) {
    let shrink = |r: f32| (r - inset).max(0.0);
    (
        shrink(radii.0),
        shrink(radii.1),
        shrink(radii.2),
        shrink(radii.3),
    )
}

/// 4辺それぞれの`border-width`/`border-style`/`border-color`に従って枠線を描く。
///
/// `solid`/`double`は、外形(border-box外周)から内形(border-box内周)まで
/// 各辺を四角形(太さが辺ごとに異なれば台形)として直接塗りつぶす。隣接する
/// 2辺は角の頂点(例: 右上なら`(x1, y_top)`と`(x1 - border.right, y_top -
/// border.top)`)を共有するため、太さ・色が異なっていても角が斜めに
/// ミトー結合される(ピクチャーフレームと同じ要領)。`dashed`/`dotted`は
/// ダッシュパターンをストロークで表現する都合上、従来通り太さの中心線を
/// ストロークする(ダッシュの境界はどのみち辺ごとに揃わないため、ミトー結合の
/// 恩恵が薄く実装コストに見合わない簡略化)。
fn render_border(
    content: &mut Content,
    layout: &Layout,
    style: &ComputedStyle,
    settings: &PageSettings,
) {
    let border_box = layout.border_box();
    let x0 = settings.margin.left + border_box.x;
    let x1 = x0 + border_box.width;
    let y_top = to_pdf_y(settings, border_box.y);
    let y_bottom = to_pdf_y(settings, border_box.y + border_box.height);
    let t = layout.border;

    let tl_outer = (x0, y_top);
    let tr_outer = (x1, y_top);
    let br_outer = (x1, y_bottom);
    let bl_outer = (x0, y_bottom);
    let tl_inner = (x0 + t.left, y_top - t.top);
    let tr_inner = (x1 - t.right, y_top - t.top);
    let br_inner = (x1 - t.right, y_bottom + t.bottom);
    let bl_inner = (x0 + t.left, y_bottom + t.bottom);

    render_border_side(
        content,
        style.border_top_style,
        style.border_top_color,
        t.top,
        BorderSideCorners::new(tl_outer, tr_outer, tr_inner, tl_inner),
    );
    render_border_side(
        content,
        style.border_right_style,
        style.border_right_color,
        t.right,
        BorderSideCorners::new(tr_outer, br_outer, br_inner, tr_inner),
    );
    render_border_side(
        content,
        style.border_bottom_style,
        style.border_bottom_color,
        t.bottom,
        BorderSideCorners::new(br_outer, bl_outer, bl_inner, br_inner),
    );
    render_border_side(
        content,
        style.border_left_style,
        style.border_left_color,
        t.left,
        BorderSideCorners::new(bl_outer, tl_outer, tl_inner, bl_inner),
    );
}

/// 1辺分の枠線を構成する4頂点。`outer_a`→`outer_b`が外形の辺、
/// `inner_b`→`inner_a`が内形の辺(`outer_b`/`inner_b`が隣の辺と共有する角)。
struct BorderSideCorners {
    outer_a: (f32, f32),
    outer_b: (f32, f32),
    inner_b: (f32, f32),
    inner_a: (f32, f32),
}

impl BorderSideCorners {
    fn new(
        outer_a: (f32, f32),
        outer_b: (f32, f32),
        inner_b: (f32, f32),
        inner_a: (f32, f32),
    ) -> Self {
        Self {
            outer_a,
            outer_b,
            inner_b,
            inner_a,
        }
    }
}

/// 1辺分の枠線を描く。
fn render_border_side(
    content: &mut Content,
    border_style: BorderStyle,
    color: RgbaColor,
    thickness: f32,
    corners: BorderSideCorners,
) {
    if thickness <= 0.0 || border_style == BorderStyle::None {
        return;
    }
    let BorderSideCorners {
        outer_a,
        outer_b,
        inner_b,
        inner_a,
    } = corners;

    match border_style {
        BorderStyle::Solid => {
            content.set_fill_rgb(
                color.red as f32 / 255.0,
                color.green as f32 / 255.0,
                color.blue as f32 / 255.0,
            );
            fill_quad(content, outer_a, outer_b, inner_b, inner_a);
        }
        BorderStyle::Double => {
            // 太さを3等分し、外側1/3・内側1/3それぞれをミトー結合済みの帯として
            // 塗る(中央の1/3は空白として残る)。外形/内形の頂点間を線形補間して
            // 各帯の境界を求める(辺ごとに太さが異なっていても、隣接辺との
            // 境界は共有する頂点から計算されるため引き続き綺麗に合う)。
            content.set_fill_rgb(
                color.red as f32 / 255.0,
                color.green as f32 / 255.0,
                color.blue as f32 / 255.0,
            );
            const BAND: f32 = 1.0 / 3.0;
            for (t0, t1) in [(0.0, BAND), (1.0 - BAND, 1.0)] {
                fill_quad(
                    content,
                    lerp(outer_a, inner_a, t0),
                    lerp(outer_b, inner_b, t0),
                    lerp(outer_b, inner_b, t1),
                    lerp(outer_a, inner_a, t1),
                );
            }
        }
        BorderStyle::Dashed | BorderStyle::Dotted => {
            // ダッシュパターンはストロークでのみ表現できるため、太さの中心線を
            // 従来通りストロークする(ミトー結合はしない)。
            content.set_stroke_rgb(
                color.red as f32 / 255.0,
                color.green as f32 / 255.0,
                color.blue as f32 / 255.0,
            );
            content.set_line_width(thickness);
            apply_border_style_dash(content, border_style, thickness);
            let from = lerp(outer_a, inner_a, 0.5);
            let to = lerp(outer_b, inner_b, 0.5);
            content.move_to(from.0, from.1);
            content.line_to(to.0, to.1);
            content.stroke();
        }
        BorderStyle::None => {}
    }
}

/// 単純な実線を太さ・色を指定してストロークする(text-decorationの下線・
/// 取り消し線用。border描画とは異なりミトー結合等は関係ない単発の直線)。
fn stroke_line(
    content: &mut Content,
    thickness: f32,
    color: RgbaColor,
    from: (f32, f32),
    to: (f32, f32),
) {
    if thickness <= 0.0 {
        return;
    }
    content.set_stroke_rgb(
        color.red as f32 / 255.0,
        color.green as f32 / 255.0,
        color.blue as f32 / 255.0,
    );
    content.set_line_width(thickness);
    content.set_line_cap(LineCapStyle::ButtCap);
    content.set_dash_pattern([], 0.0);
    content.move_to(from.0, from.1);
    content.line_to(to.0, to.1);
    content.stroke();
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// 4頂点(a→b→c→d→閉じる)の四角形パスを構築して塗りつぶす。
fn fill_quad(content: &mut Content, a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) {
    content.move_to(a.0, a.1);
    content.line_to(b.0, b.1);
    content.line_to(c.0, c.1);
    content.line_to(d.0, d.1);
    content.close_path();
    content.fill_nonzero();
}

/// `border-style`に応じたダッシュパターン/線キャップを設定する。
/// `Double`は2本ストロークする専用処理(呼び出し側)で扱うためここには来ない。
fn apply_border_style_dash(content: &mut Content, border_style: BorderStyle, thickness: f32) {
    match border_style {
        BorderStyle::Solid | BorderStyle::Double => {
            content.set_line_cap(LineCapStyle::ButtCap);
            content.set_dash_pattern([], 0.0);
        }
        BorderStyle::Dashed => {
            content.set_line_cap(LineCapStyle::ButtCap);
            content.set_dash_pattern([thickness * 3.0], 0.0);
        }
        BorderStyle::Dotted => {
            // 長さ0の破線+丸キャップで点線を表現する(PDFの定石)。
            content.set_line_cap(LineCapStyle::RoundCap);
            content.set_dash_pattern([0.01, thickness * 2.0], 0.0);
        }
        BorderStyle::None => {}
    }
}

/// 疑似イタリック(シアー変形)の傾斜角(12度)。埋め込みフォントに本物の
/// イタリック字形がない前提で、テキスト行列をせん断することで代用する。
const ITALIC_SHEAR: f32 = 0.2126; // tan(12°)
/// 疑似ボールド(塗り+縁取り)の線幅を、フォントサイズに対する比率で表す。
const BOLD_STROKE_RATIO: f32 = 0.03;

fn render_line(
    content: &mut Content,
    line: &LineBox,
    fonts: &FontCollection,
    settings: &PageSettings,
    remaps: Option<&[HashMap<u16, u16>]>,
    font_resource_names: &[String],
) {
    let Some(first_run) = line.runs.first() else {
        return;
    };

    // 行内で複数フォントが混在していても、ベースラインは先頭ランのフォント・
    // サイズのメトリクスを基準に統一する。
    let baseline_font = fonts.get(first_run.font_index);
    let baseline_offset_px = baseline_font
        .map(|f| f.baseline_offset(first_run.font_size, line.rect.height))
        .unwrap_or(first_run.font_size);
    let baseline_y = to_pdf_y(settings, line.rect.y + baseline_offset_px);

    content.begin_text();

    // ランどうしの間に、実際のグリフ幅の合計を超える隙間があれば単語境界
    // (=空白1文字分)とみなす。単語内でスタイル/フォントが切り替わる場合の
    // ラン境界は隙間0で連続しているため、ここでは誤って空白扱いにならない。
    const WORD_GAP_EPSILON: f32 = 0.01;
    let mut previous_run_end: Option<f32> = None;

    for run in &line.runs {
        if run.glyphs.is_empty() {
            continue;
        }
        // `remaps`が`Some`(一括処理)ならサブセット後のグリフIDへの変換表を
        // 引く。`None`(ストリーミング処理)ならCIDは常に元のグリフIDのまま
        // 使う([`super::font::embed_font_streaming_chunks`]参照)。
        let remap = match remaps {
            Some(remaps) => match remaps.get(run.font_index) {
                Some(remap) => Some(remap),
                None => continue,
            },
            None => None,
        };
        let Some(resource_name) = font_resource_names.get(run.font_index) else {
            continue;
        };

        // 単語間の空白は、レイアウト上は隙間(x_offsetの加算)としてのみ表現され、
        // どの`TextRun.text`にも実際の空白文字を含めていない(フォント混在時の
        // グリフ幅計測を単純にするため)。そのままではPDFからのテキスト抽出時、
        // 特にフォント(リソース名)が切り替わるラン境界で空白が失われることが
        // あるため、見た目に影響しない`ActualText`付きの空マーク付きコンテンツ
        // 区間を挿入し、抽出用にスペースの存在を明示する。
        if let Some(prev_end) = previous_run_end {
            if run.x_offset > prev_end + WORD_GAP_EPSILON {
                let mut marked = content.begin_marked_content_with_properties(Name(b"Span"));
                marked.properties().actual_text(TextStr(" "));
                marked.finish();
                content.end_marked_content();
            }
        }
        previous_run_end = Some(run.x_offset + run.width);

        let mut glyph_bytes = Vec::with_capacity(run.glyphs.len() * 2);
        for glyph in &run.glyphs {
            let cid = match remap {
                Some(remap) => remap.get(&glyph.glyph_id).copied().unwrap_or(0),
                None => glyph.glyph_id,
            };
            glyph_bytes.extend_from_slice(&cid.to_be_bytes());
        }

        content.set_fill_rgb(
            run.color.red as f32 / 255.0,
            run.color.green as f32 / 255.0,
            run.color.blue as f32 / 255.0,
        );
        if run.bold {
            content.set_stroke_rgb(
                run.color.red as f32 / 255.0,
                run.color.green as f32 / 255.0,
                run.color.blue as f32 / 255.0,
            );
            content.set_line_width(run.font_size * BOLD_STROKE_RATIO);
            // 枠線描画がダッシュパターン/丸キャップを残している場合があるため、
            // テキストの縁取りには影響しないよう明示的に実線・矩形キャップへ戻す。
            content.set_line_cap(LineCapStyle::ButtCap);
            content.set_dash_pattern([], 0.0);
            content.set_text_rendering_mode(TextRenderingMode::FillStroke);
        } else {
            content.set_text_rendering_mode(TextRenderingMode::Fill);
        }

        let x = settings.margin.left + line.rect.x + run.x_offset;
        let shear = if run.italic { ITALIC_SHEAR } else { 0.0 };
        content.set_font(Name(resource_name.as_bytes()), run.font_size);
        content.set_text_matrix([1.0, 0.0, shear, 1.0, x, baseline_y]);
        // `letter-spacing`はグリフ幅そのもの(フォントの`/Widths`)には反映
        // できないため、PDFの`Tc`(character spacing)を使う。`Tw`(word
        // spacing)と異なり複合フォント(2バイトCID)にも適用される
        // ([0020](../../../docs/decisions/0020-typography-details-design.md)
        // 決定2)。0でも明示的に設定し、前のランの値がグラフィックステートに
        // 残らないようにする。
        content.set_char_spacing(run.letter_spacing);
        content.show(pdf_writer::Str(&glyph_bytes));
    }

    content.end_text();

    for run in &line.runs {
        if !run.underline && !run.line_through {
            continue;
        }
        let Some(font) = fonts.get(run.font_index) else {
            continue;
        };
        let x = settings.margin.left + line.rect.x + run.x_offset;
        if run.underline {
            let (y, thickness) =
                decoration_metrics(font, run.font_size, font.underline_metrics(), -0.1);
            stroke_line(
                content,
                thickness,
                run.color,
                (x, baseline_y + y),
                (x + run.width, baseline_y + y),
            );
        }
        if run.line_through {
            let (y, thickness) =
                decoration_metrics(font, run.font_size, font.strikeout_metrics(), 0.3);
            stroke_line(
                content,
                thickness,
                run.color,
                (x, baseline_y + y),
                (x + run.width, baseline_y + y),
            );
        }
    }
}

/// フォントの`post`(下線)/`OS2`(取り消し線)テーブルから、ベースラインからの
/// 符号付きオフセットと線の太さをpx単位で求める。テーブルを持たないフォントでは
/// `fallback_ratio`(フォントサイズに対する比率)をアセント基準の位置として使う。
fn decoration_metrics(
    font: &crate::fonts::Font,
    font_size: f32,
    metrics: Option<(i16, i16)>,
    fallback_ratio: f32,
) -> (f32, f32) {
    let units_per_em = font.units_per_em() as f32;
    match metrics {
        Some((position, thickness)) if thickness > 0 => (
            position as f32 / units_per_em * font_size,
            thickness as f32 / units_per_em * font_size,
        ),
        _ => (font_size * fallback_ratio, font_size * 0.05),
    }
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

    /// PDFバイト列中の全`stream`〜`endstream`区間を取り出し、
    /// zlib(`/FlateDecode`)で圧縮されていれば展開して連結したものを返す。
    /// コンテンツストリームは圧縮済みなので、オペレータ列を文字列として
    /// 検証したいテストはこちらを使う(構造レベルの辞書キー、例えば
    /// `/Subtype /Type0`のような、ストリーム本体の外にある文字列は
    /// 元の`bytes`のままで検証してよい)。
    fn decompressed_stream_bytes(pdf_bytes: &[u8]) -> Vec<u8> {
        fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack.windows(needle.len()).position(|w| w == needle)
        }

        let mut out = Vec::new();
        let mut i = 0;
        while let Some(pos) = find_subslice(&pdf_bytes[i..], b"stream\n") {
            let start = i + pos + b"stream\n".len();
            let Some(end_rel) = find_subslice(&pdf_bytes[start..], b"\nendstream") else {
                break;
            };
            let end = start + end_rel;
            let raw = &pdf_bytes[start..end];

            let mut decoder = flate2::read::ZlibDecoder::new(raw);
            let mut decompressed = Vec::new();
            if std::io::Read::read_to_end(&mut decoder, &mut decompressed).is_ok() {
                out.extend_from_slice(&decompressed);
            } else {
                out.extend_from_slice(raw);
            }
            out.push(b'\n');

            i = end + b"\nendstream".len();
        }
        out
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
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

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
        assert!(
            count_occurrences(&bytes, b"/FlateDecode") > 0,
            "font stream should be compressed"
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
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

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

        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
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
        let bytes_with = encode_pdf(
            &pages_with,
            &styles_with,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        let dom_without_bg = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without_bg, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without_bg, &styles_without, &fonts, &settings);
        let bytes_without = encode_pdf(
            &pages_without,
            &styles_without,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        assert!(
            bytes_with.len() > bytes_without.len(),
            "background-color should add extra drawing operators to the content stream"
        );
    }

    #[test]
    fn solid_border_fills_a_mitered_quad_per_side() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom_with = html::parse(br#"<div class="box">x</div>"#);
        let author_with = parse_stylesheet(".box { border: 2px solid rgb(10, 20, 30); }");
        let styles_with = compute_styles(&dom_with, &ua, &author_with);
        let pages_with = paginate_document(&dom_with, &styles_with, &fonts, &settings);
        let bytes_with = encode_pdf(
            &pages_with,
            &styles_with,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        let dom_without = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without, &styles_without, &fonts, &settings);
        let bytes_without = encode_pdf(
            &pages_without,
            &styles_without,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        // 4辺分の塗りつぶし(`f`オペレータ)が追加されているはず(各辺は
        // 外形/内形の頂点を結ぶミトー結合済みの四角形として塗る)。
        let fill_count_with = count_occurrences(&decompressed_stream_bytes(&bytes_with), b"\nf\n");
        let fill_count_without =
            count_occurrences(&decompressed_stream_bytes(&bytes_without), b"\nf\n");
        assert!(
            fill_count_with >= fill_count_without + 4,
            "solid border should add 4 filled mitered quads (with={fill_count_with}, without={fill_count_without})"
        );
    }

    #[test]
    fn text_decoration_underline_adds_stroke_operator() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom_decorated = html::parse(br#"<p class="u">underlined</p>"#);
        let author = parse_stylesheet(".u { text-decoration: underline; }");
        let styles_decorated = compute_styles(&dom_decorated, &ua, &author);
        let pages_decorated =
            paginate_document(&dom_decorated, &styles_decorated, &fonts, &settings);
        let bytes_decorated = encode_pdf(
            &pages_decorated,
            &styles_decorated,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        let dom_plain = html::parse(br#"<p class="u">underlined</p>"#);
        let styles_plain = compute_styles(&dom_plain, &ua, &Stylesheet::default());
        let pages_plain = paginate_document(&dom_plain, &styles_plain, &fonts, &settings);
        let bytes_plain = encode_pdf(
            &pages_plain,
            &styles_plain,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        assert!(
            count_occurrences(&decompressed_stream_bytes(&bytes_decorated), b"\nS\n")
                > count_occurrences(&decompressed_stream_bytes(&bytes_plain), b"\nS\n"),
            "underline should add an extra stroke operator to the content stream"
        );
    }

    #[test]
    fn double_border_fills_two_bands_per_side() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom_with = html::parse(br#"<div class="box">x</div>"#);
        let author_with = parse_stylesheet(".box { border: 9px double rgb(0, 0, 0); }");
        let styles_with = compute_styles(&dom_with, &ua, &author_with);
        let pages_with = paginate_document(&dom_with, &styles_with, &fonts, &settings);
        let bytes_with = encode_pdf(
            &pages_with,
            &styles_with,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        let dom_without = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without, &styles_without, &fonts, &settings);
        let bytes_without = encode_pdf(
            &pages_without,
            &styles_without,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        // 4辺 x 2帯(外側/内側) = 8回以上の塗りつぶしが追加されているはず。
        let fill_count_with = count_occurrences(&decompressed_stream_bytes(&bytes_with), b"\nf\n");
        let fill_count_without =
            count_occurrences(&decompressed_stream_bytes(&bytes_without), b"\nf\n");
        assert!(
            fill_count_with >= fill_count_without + 8,
            "double border should fill two mitered bands per side"
        );
    }

    #[test]
    fn double_border_with_radius_strokes_two_rounded_paths() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom = html::parse(br#"<div class="box">x</div>"#);
        let author =
            parse_stylesheet(".box { border: 9px double rgb(0, 0, 0); border-radius: 10px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        // 角丸パス(4角ぶんのベジェ曲線)を2周分ストロークするはず(背景色は
        // 未指定なので塗りつぶしはなし)。
        let decompressed = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&decompressed, b" c\n") >= 8,
            "double border with radius should draw two rounded stroke paths"
        );
        assert!(
            count_occurrences(&decompressed, b"\nS\n") >= 2,
            "double border with radius should stroke twice"
        );
    }

    #[test]
    fn dotted_border_uses_round_cap_and_dash_pattern() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom = html::parse(br#"<div class="box">x</div>"#);
        let author = parse_stylesheet(".box { border: 1px dotted rgb(0, 0, 0); }");
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
        let text = String::from_utf8_lossy(&decompressed_stream_bytes(&bytes)).into_owned();

        assert!(text.contains(" J\n"), "dotted border should set a line cap");
        assert!(
            text.contains(" d\n"),
            "dotted border should set a dash pattern"
        );
    }

    #[test]
    fn uniform_border_radius_draws_curved_path_instead_of_straight_rect() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom = html::parse(br#"<div class="box">x</div>"#);
        let author = parse_stylesheet(
            ".box { border: 2px solid rgb(0, 0, 0); background-color: rgb(200, 200, 200); border-radius: 10px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
        let decompressed = decompressed_stream_bytes(&bytes);
        let text = String::from_utf8_lossy(&decompressed);

        // 角丸パスはベジェ曲線オペレータ`c`を使う。
        assert!(
            count_occurrences(&decompressed, b" c\n") >= 8,
            "rounded corners should use cubic bezier curve operators (4 corners x fill+stroke)"
        );
        // 直線矩形の`re`は(角丸なので)使われないはず。
        assert!(
            !text.contains(" re\n"),
            "rounded box should not use a plain rectangle"
        );
    }

    #[test]
    fn non_uniform_border_with_radius_falls_back_to_straight_edges() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom = html::parse(br#"<div class="box">x</div>"#);
        let author = parse_stylesheet(
            ".box { border-style: solid dotted; border-width: 2px; border-color: rgb(0,0,0); border-radius: 10px; }",
        );
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        // 4辺が不揃いなので角丸は諦め、直線4辺のフォールバックになるはず。
        // `border-style: solid dotted`は上下がsolid(塗り)、左右がdotted
        // (ストローク)に展開されるので、両方が現れるはず。
        let decompressed = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&decompressed, b"\nf\n") >= 2,
            "the two solid sides should fill mitered quads"
        );
        assert!(
            count_occurrences(&decompressed, b"\nS\n") >= 2,
            "the two dotted sides should still stroke a centerline"
        );
    }

    #[test]
    fn non_uniform_solid_border_corners_share_exact_miter_vertices() {
        use crate::layout::{EdgeSizes, PageSize};

        // ページ余白0・丸い数値のPageSettingsを使い、座標を手計算で予測できる
        // ようにする。4辺の太さ・色をすべて不揃いにし、隣接する2辺が
        // 「内側の角の頂点」を正確に共有する(=斜めにミトー結合される)ことを、
        // 生成された実際のコンテンツストリームの座標列で確認する。
        let settings = PageSettings {
            size: PageSize {
                width: 800.0,
                height: 1000.0,
            },
            margin: EdgeSizes {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };
        let fonts = test_fonts();

        let dom = html::parse(br#"<div class="box">x</div>"#);
        let author = parse_stylesheet(
            "html, body { margin: 0; } \
             .box { border-style: solid; border-width: 10px 20px 30px 40px; \
             border-color: rgb(255,0,0) rgb(0,255,0) rgb(0,0,255) rgb(255,255,0); \
             width: 300px; height: 200px; margin: 0; }",
        );
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
        let text = String::from_utf8_lossy(&decompressed_stream_bytes(&bytes)).into_owned();

        // border-box: x∈[0,360](border-left 40 + width 300 + border-right 20)、
        // PDF空間でy_top=1000(border-top 10)、y_bottom=760(border-bottom 30)。
        // 右上の外側の角(360,1000)と内側の角(340,990)は、top/rightの両方の
        // パスに現れるはず(top側は終端、right側は始端として)。
        assert_eq!(
            count_occurrences(text.as_bytes(), b"360 1000"),
            2,
            "the top-right outer corner should be shared by the top and right quads"
        );
        assert_eq!(
            count_occurrences(text.as_bytes(), b"340 990"),
            2,
            "the top-right inner (mitered) corner should be shared by the top and right quads"
        );
    }

    #[test]
    fn border_style_none_suppresses_drawing_even_with_nonzero_width() {
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom_with = html::parse(br#"<div class="box">x</div>"#);
        let author_with = parse_stylesheet(".box { border-width: 5px; border-style: none; }");
        let styles_with = compute_styles(&dom_with, &ua, &author_with);
        let pages_with = paginate_document(&dom_with, &styles_with, &fonts, &settings);
        let bytes_with = encode_pdf(
            &pages_with,
            &styles_with,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        let dom_without = html::parse(br#"<div class="box">x</div>"#);
        let styles_without = compute_styles(&dom_without, &ua, &Stylesheet::default());
        let pages_without = paginate_document(&dom_without, &styles_without, &fonts, &settings);
        let bytes_without = encode_pdf(
            &pages_without,
            &styles_without,
            &HashMap::new(),
            &fonts,
            &settings,
        );

        assert_eq!(
            bytes_with.len(),
            bytes_without.len(),
            "border-style: none should suppress drawing regardless of border-width"
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
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        // 2つのフォント(DejaVu Sans, Noto Sans CJK JP)がそれぞれ埋め込まれているはず。
        assert_eq!(count_occurrences(&bytes, b"/FontFile2"), 2);
        assert_eq!(count_occurrences(&bytes, b"/Subtype /Type0"), 2);
    }

    #[test]
    fn table_cells_render_text_borders_and_backgrounds() {
        let dom = html::parse(
            br#"<table>
                <tr><th colspan="2">Header</th></tr>
                <tr><td style="background-color: rgb(200,200,200);">Apple</td><td>100</td></tr>
            </table>"#,
        );
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet("td, th { border: 1px solid rgb(0,0,0); }");
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
        let decompressed = decompressed_stream_bytes(&bytes);
        let text = String::from_utf8_lossy(&decompressed);

        // 各セルのテキストがコンテンツストリームに(グリフとして)出力されている
        // ことを、フォント使用状況(グリフ数)経由で間接的に確認する。
        // "Header"/"Apple"/"100"のテキストが1つのフォントに集約されているはず
        // なので、埋め込みフォントは1つだけ。
        assert_eq!(
            count_occurrences(&bytes, b"/FontFile2"),
            1,
            "all table cell text should use the single loaded font"
        );

        // colspanで結合されたヘッダーセルの背景・枠線と、通常セルの背景・枠線を
        // 合わせて複数の塗りつぶし(`f`)が出力されているはず(テーブル自身には
        // 背景/枠線を指定していないので、セル由来のみ)。
        assert!(
            count_occurrences(&decompressed, b"\nf\n") >= 2,
            "cell borders/backgrounds should produce fill operators"
        );
        // 明示的に指定したセル背景色がfillの色として現れるはず。
        assert!(
            text.contains("0.78431374 0.78431374 0.78431374 rg"),
            "the explicit cell background-color should be painted"
        );
    }

    /// 与えたHTML/CSSをPDF化し、展開したコンテンツストリーム中の塗りつぶし
    /// (`f`)演算子の出現数を返す(背景・枠線描画の合計を数える簡易プロキシ)。
    fn fill_operator_count(html_src: &str, css: &str) -> usize {
        let dom = html::parse(html_src.as_bytes());
        let ua = user_agent_stylesheet();
        let author = parse_stylesheet(css);
        let styles = compute_styles(&dom, &ua, &author);
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);
        count_occurrences(&decompressed_stream_bytes(&bytes), b"\nf\n")
    }

    #[test]
    fn empty_cells_hide_suppresses_decoration_for_empty_cells_in_separate_mode() {
        let html_src = r#"<table><tr><td>Apple</td><td></td></tr></table>"#;
        let base_css = "td { border: 1px solid black; background-color: rgb(200,200,200); }";

        let shown = fill_operator_count(html_src, base_css);
        let hidden = fill_operator_count(
            html_src,
            &format!("{base_css} table {{ empty-cells: hide; }}"),
        );

        assert!(
            hidden < shown,
            "hiding the empty cell should remove its border/background fills \
             (shown={shown}, hidden={hidden})"
        );
    }

    #[test]
    fn empty_cells_hide_has_no_effect_when_border_collapse_is_collapse() {
        let html_src = r#"<table><tr><td>Apple</td><td></td></tr></table>"#;
        let base_css = "td { border: 1px solid black; background-color: rgb(200,200,200); } \
             table { border-collapse: collapse; }";

        let without_hide = fill_operator_count(html_src, base_css);
        let with_hide = fill_operator_count(
            html_src,
            &format!("{base_css} table {{ empty-cells: hide; }}"),
        );

        assert_eq!(
            without_hide, with_hide,
            "empty-cells: hide should be a no-op under border-collapse: collapse"
        );
    }

    #[test]
    fn empty_cells_hide_can_be_set_on_an_individual_cell() {
        // テーブル自身はデフォルト(show)のまま、空セルにだけ`empty-cells: hide`
        // を指定した場合でもそのセルの装飾が抑制されることを確認する
        // (このプロパティは`table-cell`要素に適用されるため、テーブル単位では
        // なくセル単位で見る必要がある)。
        let html_src = r#"<table><tr><td>Apple</td><td class="empty"></td></tr></table>"#;
        let base_css = "td { border: 1px solid black; background-color: rgb(200,200,200); }";

        let shown = fill_operator_count(html_src, base_css);
        let hidden = fill_operator_count(
            html_src,
            &format!("{base_css} .empty {{ empty-cells: hide; }}"),
        );

        assert!(
            hidden < shown,
            "hiding via a per-cell override should remove that cell's fills \
             (shown={shown}, hidden={hidden})"
        );
    }

    #[test]
    fn border_collapse_avoids_drawing_a_double_thick_border_at_a_shared_edge() {
        // 隣接する2セルが同じ枠線を指定している場合、separateモデルでは
        // 各セルが独立に4辺とも描画する(2+2セル分=8回)。collapseモデルでは
        // 内部で接する1辺の描画が抑制されて1回に統合されるため、合計は1回
        // 減った7回になるはず([0021]決定1・2)。
        let html_src = r#"<table><tr><td>a</td><td>b</td></tr></table>"#;
        let base_css = "body { margin: 0; } td { border: 1px solid black; }";

        let separate = fill_operator_count(html_src, base_css);
        let collapse = fill_operator_count(
            html_src,
            &format!("{base_css} table {{ border-collapse: collapse; }}"),
        );

        assert_eq!(
            separate, 8,
            "each cell should draw all 4 sides independently in separate mode"
        );
        assert_eq!(
            collapse, 7,
            "collapse should merge the shared edge into a single draw (8-1=7): {collapse}"
        );
    }

    #[test]
    fn border_collapse_uses_the_neighbors_border_when_own_side_declares_none() {
        // 左セルは枠線を指定していない(none)が、右セルの左辺(実際には隣接
        // する境界の統合先である左セルの右辺として解決される)に実際の枠線が
        // あるため、境界に枠線が現れなくなってはいけない
        // (「own=none」を無条件に採用してはいけないことの回帰テスト)。
        let html_src = r#"<table><tr><td class="a">a</td><td class="b">b</td></tr></table>"#;
        let css = "body { margin: 0; } \
                   table { border-collapse: collapse; } \
                   .a { border: none; } \
                   .b { border: 2px solid black; }";

        let fills = fill_operator_count(html_src, css);
        // 右セルの上/右/下辺(3, 左辺は隣接があるため抑制)+左セルの右辺
        // (隣接セルの枠線を継承して1)=合計4のはず。
        assert_eq!(
            fills, 4,
            "the shared edge should still be drawn using the neighbor's border spec: {fills}"
        );
    }

    #[test]
    fn resolve_border_conflict_prefers_the_wider_border() {
        let wide = (
            3.0,
            BorderStyle::Solid,
            RgbaColor {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        let narrow = (
            1.0,
            BorderStyle::Solid,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 255,
                alpha: 255.0,
            },
        );
        assert_eq!(resolve_border_conflict(wide, narrow), wide);
        assert_eq!(resolve_border_conflict(narrow, wide), wide);
    }

    #[test]
    fn resolve_border_conflict_prefers_a_stronger_style_when_widths_tie() {
        let solid = (
            1.0,
            BorderStyle::Solid,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        let dotted = (
            1.0,
            BorderStyle::Dotted,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        let double = (
            1.0,
            BorderStyle::Double,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        assert_eq!(resolve_border_conflict(solid, dotted), solid);
        assert_eq!(resolve_border_conflict(double, solid), double);
    }

    #[test]
    fn resolve_border_conflict_ignores_a_declared_width_when_style_is_none() {
        // `style: none`の辺は幅の指定に関わらず実効幅0として扱われるため、
        // 幅の数値だけを見れば「勝って」しまいそうな場合でも負けるはず。
        let none_but_wide = (
            10.0,
            BorderStyle::None,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        let thin_solid = (
            1.0,
            BorderStyle::Solid,
            RgbaColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255.0,
            },
        );
        let a = border_edge(none_but_wide.0, none_but_wide.1, none_but_wide.2);
        let b = border_edge(thin_solid.0, thin_solid.1, thin_solid.2);
        assert_eq!(resolve_border_conflict(a, b), b);
    }

    #[test]
    fn word_boundary_across_a_font_switch_gets_an_actual_text_space_marker() {
        // "Invoice"(DejaVu)と"請求書"(CJK)はフォントが切り替わるラン境界に
        // またがる単語境界で、どちらのTextRun.textにも実際の空白文字を含まない
        // (単語間の空白はx_offsetの隙間としてのみ表現される)。座標ギャップに
        // 頼るテキスト抽出はフォント切り替えを伴う境界で崩れることがあるため、
        // 視覚描画に影響しない`ActualText`付きマーク区間で明示しているはず。
        let dom = html::parse("<p>Invoice 請求書</p>".as_bytes());
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let fonts = test_fonts_with_cjk();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        assert!(
            count_occurrences(&decompressed_stream_bytes(&bytes), b"/ActualText") > 0,
            "a word boundary spanning a font switch should get an ActualText space marker"
        );
    }

    #[test]
    fn single_word_does_not_insert_an_actual_text_marker() {
        let dom = html::parse(b"<p>hello</p>");
        let ua = user_agent_stylesheet();
        let styles = compute_styles(&dom, &ua, &Stylesheet::default());
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        assert_eq!(
            count_occurrences(&decompressed_stream_bytes(&bytes), b"/ActualText"),
            0,
            "a single word with no boundary needs no ActualText marker"
        );
    }

    #[test]
    fn letter_spacing_emits_a_tc_operator_with_the_resolved_value() {
        // `letter-spacing`はグリフ幅そのものには反映できないため、PDFの`Tc`
        // (character spacing)演算子として出力される必要がある([0020]決定2)。
        let ua = user_agent_stylesheet();
        let fonts = test_fonts();
        let settings = PageSettings::default();

        let dom = html::parse(br#"<p class="s">spaced</p>"#);
        let author = parse_stylesheet(".s { letter-spacing: 3px; }");
        let styles = compute_styles(&dom, &ua, &author);
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        let bytes = encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings);

        let stream = decompressed_stream_bytes(&bytes);
        assert!(
            count_occurrences(&stream, b" Tc\n") > 0,
            "letter-spacing should emit a Tc operator"
        );
        assert!(
            count_occurrences(&stream, b"3 Tc\n") > 0,
            "the Tc operand should match the resolved letter-spacing value"
        );
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

        let bytes = write_document(
            &pages,
            &styles,
            &HashMap::new(),
            &fonts,
            &settings,
            MemorySink::new(),
        )
        .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
