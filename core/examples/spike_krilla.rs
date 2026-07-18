//! T1スパイク: krillaで最小PDF(矩形+1行テキスト)を生成するPoC。
//!
//! 観察点: `Document`はページを内部に溜め込み、`finish()`を一度呼ぶまで
//! バイト列を取り出す手段がない。つまりページ確定ごとに部分的なバイト列を
//! Sinkへflushする、というM1以降の設計方針とは相性が悪い(ドキュメント全体を
//! メモリに保持し続ける必要がある)。
//!
//! 実行: `cargo run --example spike_krilla`
//! (ローカルのDejaVu Sansフォントに依存。本実装のフォント解決はPhase 7で設計する)

use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::text::{Font, TextDirection};
use krilla::Document;

const FONT_PATH: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

fn main() {
    let font_data = std::fs::read(FONT_PATH)
        .unwrap_or_else(|e| panic!("フォント読み込みに失敗しました: {FONT_PATH}: {e}"));
    let font = Font::new(font_data.into(), 0).expect("フォントの解析に失敗しました");

    let mut document = Document::new();

    for page_no in 1..=2 {
        let mut page = document.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
        let mut surface = page.surface();

        surface.set_fill(Some(Fill {
            paint: rgb::Color::new(200, 200, 200).into(),
            opacity: NormalizedF32::ONE,
            rule: Default::default(),
        }));
        let mut pb = PathBuilder::new();
        pb.push_rect(Rect::from_xywh(20.0, 20.0, 100.0, 50.0).unwrap());
        surface.draw_path(&pb.finish().unwrap());

        surface.set_fill(Some(Fill {
            paint: rgb::Color::new(0, 0, 0).into(),
            opacity: NormalizedF32::ONE,
            rule: Default::default(),
        }));
        surface.draw_text(
            Point::from_xy(20.0, 100.0),
            font.clone(),
            14.0,
            &format!("Hello from krilla, page {page_no}"),
            false,
            TextDirection::Auto,
        );

        surface.finish();
        page.finish();
    }

    // この時点まで全ページがDocument内部に保持されたままで、
    // finish()を呼んで初めて完成したPDF全体のバイト列を得られる。
    let pdf = document.finish().expect("PDF生成に失敗しました");

    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/spike_krilla.pdf");
    std::fs::write(&out, &pdf).unwrap();
    eprintln!("wrote {} bytes to {}", pdf.len(), out.display());
}
