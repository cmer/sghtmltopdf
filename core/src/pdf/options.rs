//! PDF書き出しの振る舞いを変えるオプション。
//!
//! レイアウト結果は変えず、PDFの書き出し方だけを変える設定をまとめて
//! 持ち回るための型。CLIの`--title`/`--no-pdf-compression`/`--grayscale`/
//! `--dpi`/`--zoom`がここへ集約される。

/// PDF Info辞書に書く文書メタデータ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentMetadata {
    /// `--title`。未指定ならHTMLの`<title>`が入る(呼び出し側で解決する)。
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
}

impl DocumentMetadata {
    /// `title`が未設定のときだけHTMLの`<title>`を採用する。
    pub fn fill_title_from_document(&mut self, document_title: Option<String>) {
        if self.title.is_none() {
            self.title = document_title.filter(|t| !t.trim().is_empty());
        }
    }
}

/// CSS pxをPDFのptへ換算する既定の係数(96dpi基準、`72 / 96`)。
pub const DEFAULT_SCALE: f32 = 72.0 / 96.0;

/// PDF書き出しオプション。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfOutputOptions {
    pub metadata: DocumentMetadata,
    /// PDFオブジェクト(content stream・フォント・CMap)のFlate圧縮。
    /// 画像データはこのフラグの対象外。
    pub compress: bool,
    /// CSS px → PDF ptの係数。
    pub scale: f32,
    /// 塗り・線の色をグレースケール化する。
    pub grayscale: bool,
    /// ヘッダーの下に罫線を引く(`--header-line`)。
    pub header_line: bool,
    /// フッターの上に罫線を引く(`--footer-line`)。
    pub footer_line: bool,
}

impl Default for PdfOutputOptions {
    fn default() -> Self {
        Self {
            metadata: DocumentMetadata::default(),
            compress: true,
            scale: DEFAULT_SCALE,
            grayscale: false,
            header_line: false,
            footer_line: false,
        }
    }
}

impl PdfOutputOptions {
    /// `--dpi`と`--zoom`から換算係数を求める。
    ///
    /// `dpi`は「CSS pxを何dpiとして解釈するか」。既定の96dpiで0.75になり、
    /// 72を渡すと1 CSS px = 1 ptになる。
    pub fn scale_from_dpi_and_zoom(dpi: f32, zoom: f32) -> f32 {
        72.0 / dpi * zoom
    }

    /// ページ座標系で直接書く値(MediaBox・注釈のRect・Dests座標)の換算。
    pub fn to_pt(&self, px: f32) -> f32 {
        px * self.scale
    }

    /// 輝度式。`grayscale`が無効ならそのまま返す。
    pub fn map_rgb(&self, rgb: (f32, f32, f32)) -> (f32, f32, f32) {
        if !self.grayscale {
            return rgb;
        }
        let (r, g, b) = rgb;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        (y, y, y)
    }
}

/// PDF Info辞書の`/Producer`に書く値。
pub fn producer_string() -> String {
    format!("sghtmltopdf {}", env!("CARGO_PKG_VERSION"))
}

/// 現在時刻をPDFの日付文字列(`D:YYYYMMDDHHmmSSZ`)で返す。
/// システム時刻が取れない場合はUNIXエポックにフォールバックする。
pub fn current_pdf_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    pdf_date_from_unix(secs)
}

/// UNIX秒をPDFの日付文字列へ変換する(UTC固定)。
///
/// 依存を増やさないため、日付計算はHoward Hinnantの`civil_from_days`を
/// 自前で持つ。
pub fn pdf_date_from_unix(secs: i64) -> String {
    let (year, month, day, hour, minute, second) = datetime_from_unix(secs);
    format!("D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}Z")
}

/// UNIX秒をUTCの年月日時分秒へ分解する(`pdf_writer::Date`の組み立て用)。
pub fn datetime_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// 現在時刻をUTCの年月日時分秒で返す。
pub fn current_datetime() -> (i64, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    datetime_from_unix(secs)
}

/// エポックからの日数(1970-01-01 = 0)をグレゴリオ暦の年月日へ変換する。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scale_turns_a4_into_the_real_paper_size() {
        let options = PdfOutputOptions::default();
        // A4は793.7 × 1122.5 CSS px。ptに直すと595.3 × 841.9(=210 × 297mm)。
        assert!((options.to_pt(793.7) - 595.275).abs() < 0.1);
        assert!((options.to_pt(1122.5) - 841.875).abs() < 0.1);
    }

    #[test]
    fn dpi_72_keeps_one_css_px_as_one_pt() {
        assert!((PdfOutputOptions::scale_from_dpi_and_zoom(72.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((PdfOutputOptions::scale_from_dpi_and_zoom(96.0, 1.0) - 0.75).abs() < 1e-6);
        // zoomは係数に掛かる。
        assert!((PdfOutputOptions::scale_from_dpi_and_zoom(96.0, 2.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn grayscale_maps_colors_to_their_luminance() {
        let mut options = PdfOutputOptions::default();
        assert_eq!(options.map_rgb((1.0, 0.0, 0.0)), (1.0, 0.0, 0.0));

        options.grayscale = true;
        let (r, g, b) = options.map_rgb((1.0, 0.0, 0.0));
        assert_eq!((r, g), (b, b));
        assert!((r - 0.2126).abs() < 1e-6);
        // 白と黒は変わらない。
        assert_eq!(options.map_rgb((0.0, 0.0, 0.0)), (0.0, 0.0, 0.0));
        let (w, _, _) = options.map_rgb((1.0, 1.0, 1.0));
        assert!((w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pdf_dates_are_formatted_in_utc() {
        assert_eq!(pdf_date_from_unix(0), "D:19700101000000Z");
        // 2026-07-25T00:34:56Z
        assert_eq!(pdf_date_from_unix(1_784_939_696), "D:20260725003456Z");
        // うるう日。
        assert_eq!(pdf_date_from_unix(1_709_164_800), "D:20240229000000Z");
    }

    #[test]
    fn the_document_title_is_only_used_when_the_option_is_absent() {
        let mut meta = DocumentMetadata::default();
        meta.fill_title_from_document(Some("HTMLのtitle".to_string()));
        assert_eq!(meta.title.as_deref(), Some("HTMLのtitle"));

        let mut meta = DocumentMetadata {
            title: Some("CLI指定".to_string()),
            ..Default::default()
        };
        meta.fill_title_from_document(Some("HTMLのtitle".to_string()));
        assert_eq!(meta.title.as_deref(), Some("CLI指定"));

        // 空の<title>は採用しない。
        let mut meta = DocumentMetadata::default();
        meta.fill_title_from_document(Some("   ".to_string()));
        assert_eq!(meta.title, None);
    }
}
