//! フォントファイルの読み込み。
//!
//! M1ではローカルパス指定の最小実装のみ対応する。システムフォント探索や
//! `@font-face`によるwebfont解決は将来のマイルストーンで扱う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use self_cell::self_cell;

/// `rustybuzz::Face`のライフタイムを`self_cell`へ渡すための型構築子。
type FaceView<'a> = rustybuzz::Face<'a>;

self_cell!(
    /// フォントのバイト列と、そこから作った`rustybuzz::Face`を一緒に持つ。
    ///
    /// `Face`はバイト列を借用するため、素直に構造体へ入れると自己参照になる。
    /// `Face`の構築はフォント全体のパースを伴い1回あたり数マイクロ秒かかるので、
    /// 呼び出しのたびに作り直すとレイアウトが処理時間の大半を占めてしまう。
    struct OwnedFace {
        owner: Vec<u8>,
        #[covariant]
        dependent: FaceView,
    }
);

/// 読み込み済みのフォントデータ。
///
/// ファイルの生バイト列と、そこから構築した`rustybuzz::Face`を保持する。
/// 値の変わらないメトリクスは[`Metrics`]として構築時に1度だけ読み、
/// グリフ検索は[`Font::glyphs`]でメモ化する。
pub struct Font {
    face: OwnedFace,
    index: u32,
    metrics: Metrics,
    /// 文字 → グリフID(cmapに無ければ`None`)のメモ。
    ///
    /// 文書に現れる異なり文字数は多くないので、素直な`HashMap`で十分に効く。
    /// 内容はフォントから決まるためキャッシュとして透過的で、外から観測できる
    /// 振る舞いは変わらない。
    glyphs: RefCell<HashMap<char, Option<u16>>>,
}

impl Clone for Font {
    /// バイト列を複製して`Face`を作り直す(`Face`は複製元のバイト列を
    /// 借用しているため、そのままは持ち出せない)。
    fn clone(&self) -> Self {
        Self::from_bytes(self.data().to_vec(), self.index)
            .expect("複製元が有効なフォントなので失敗しない")
    }
}

impl fmt::Debug for Font {
    /// `rustybuzz::Face`が`Debug`を実装しないため、識別に足る情報だけ出す。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("family_name", &self.metrics.family_name)
            .field("index", &self.index)
            .field("bytes", &self.data().len())
            .finish()
    }
}

/// フォントから1度だけ読めば足りるメトリクス。
#[derive(Debug, Clone)]
struct Metrics {
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    capital_height: Option<i16>,
    x_height: Option<i16>,
    subscript_y_offset: Option<i16>,
    superscript_y_offset: Option<i16>,
    italic_angle: f32,
    is_italic: bool,
    underline: Option<(i16, i16)>,
    strikeout: Option<(i16, i16)>,
    is_monospaced: bool,
    weight: u16,
    bounding_box: ttf_parser::Rect,
    family_name: Option<String>,
}

impl Metrics {
    fn read(face: &rustybuzz::Face<'_>) -> Self {
        let names = face.names();
        let pick_name = |id: u16| {
            names
                .into_iter()
                .find(|n| n.name_id == id && n.is_unicode())
                .and_then(|n| n.to_string())
        };
        Self {
            units_per_em: face.units_per_em() as u16,
            ascender: face.ascender(),
            descender: face.descender(),
            capital_height: face.capital_height(),
            x_height: face.x_height(),
            subscript_y_offset: face.subscript_metrics().map(|m| m.y_offset),
            superscript_y_offset: face.superscript_metrics().map(|m| m.y_offset),
            italic_angle: face.italic_angle(),
            is_italic: face.is_italic(),
            underline: face.underline_metrics().map(|m| (m.position, m.thickness)),
            strikeout: face.strikeout_metrics().map(|m| (m.position, m.thickness)),
            is_monospaced: face.is_monospaced(),
            weight: face.weight().to_number(),
            bounding_box: face.global_bounding_box(),
            family_name: pick_name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
                .or_else(|| pick_name(ttf_parser::name_id::FAMILY)),
        }
    }
}

#[derive(Debug)]
pub struct FontLoadError(String);

impl fmt::Display for FontLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "フォントの読み込みに失敗しました: {}", self.0)
    }
}

impl std::error::Error for FontLoadError {}

impl Font {
    /// ローカルファイルパスからフォントを読み込む。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FontLoadError> {
        Self::load_indexed(path, 0)
    }

    /// ローカルファイルパスからフォントを読み込む。TrueType Collection(`.ttc`)等、
    /// 複数フェイスを含むファイルの場合は`index`でフェイスを選択する。
    pub fn load_indexed(path: impl AsRef<Path>, index: u32) -> Result<Self, FontLoadError> {
        let path = path.as_ref();
        let data =
            std::fs::read(path).map_err(|e| FontLoadError(format!("{}: {e}", path.display())))?;
        Self::from_bytes(data, index)
    }

    /// 読み込み済みのバイト列からフォントを構築する(TrueType Collection等、
    /// 複数フェイスを含む場合は`index`でフェイスを選択する)。
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, FontLoadError> {
        let face = OwnedFace::try_new(data, |data| {
            rustybuzz::Face::from_slice(data, index)
                .ok_or_else(|| FontLoadError("不正なフォントデータです".to_string()))
        })?;
        let metrics = Metrics::read(face.borrow_dependent());
        Ok(Self {
            face,
            index,
            metrics,
            glyphs: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn face(&self) -> &rustybuzz::Face<'_> {
        self.face.borrow_dependent()
    }

    /// フォントファイルの生バイト列(PDFへのフォント埋め込み等で必要)。
    pub fn data(&self) -> &[u8] {
        self.face.borrow_owner()
    }

    /// TrueType Collection(`.ttc`)等、複数フェイスを含むファイル内でのフェイス番号。
    pub fn face_index(&self) -> u32 {
        self.index
    }

    pub fn units_per_em(&self) -> u16 {
        self.metrics.units_per_em
    }

    pub fn ascender(&self) -> i16 {
        self.metrics.ascender
    }

    pub fn descender(&self) -> i16 {
        self.metrics.descender
    }

    pub fn capital_height(&self) -> Option<i16> {
        self.metrics.capital_height
    }

    /// アセント/ディセントから、行ボックス上端からベースラインまでの距離を
    /// 求める(フォントのem矩形を行ボックス内で上下中央に配置する)。
    /// テーブルセルの`vertical-align: baseline`(セル内容の最初の行の
    /// ベースライン位置を求める)とテキスト描画(`render_line`)の両方で使う。
    pub fn baseline_offset(&self, font_size: f32, line_height: f32) -> f32 {
        let units_per_em = self.units_per_em() as f32;
        let ascent = self.ascender() as f32 / units_per_em * font_size;
        let descent = -(self.descender() as f32) / units_per_em * font_size;
        let half_leading = (line_height - (ascent + descent)) / 2.0;
        ascent + half_leading
    }

    /// x-height(px)。`OS/2`テーブルが持たない場合はアセントの半分で近似する
    /// (`vertical-align: middle`の基準)。
    pub fn x_height(&self, font_size: f32) -> f32 {
        let units_per_em = self.units_per_em() as f32;
        match self.metrics.x_height {
            Some(x) => x as f32 / units_per_em * font_size,
            None => self.ascender() as f32 / units_per_em * font_size * 0.5,
        }
    }

    /// `vertical-align: sub`の下げ幅(px、正の値)。フォントの`OS/2`が
    /// subscriptのYオフセットを持たない場合は`0.2em`で近似する。
    pub fn subscript_offset(&self, font_size: f32) -> f32 {
        let units_per_em = self.units_per_em() as f32;
        match self.metrics.subscript_y_offset {
            Some(y_offset) => y_offset as f32 / units_per_em * font_size,
            None => font_size * 0.2,
        }
    }

    /// `vertical-align: super`の上げ幅(px、正の値)。持たない場合は`0.33em`。
    pub fn superscript_offset(&self, font_size: f32) -> f32 {
        let units_per_em = self.units_per_em() as f32;
        match self.metrics.superscript_y_offset {
            Some(y_offset) => y_offset as f32 / units_per_em * font_size,
            None => font_size * 0.33,
        }
    }

    pub fn italic_angle(&self) -> f32 {
        self.metrics.italic_angle
    }

    pub fn is_italic(&self) -> bool {
        self.metrics.is_italic
    }

    /// 下線の中心位置(ベースラインからの符号付きオフセット、フォントユニット。
    /// 上方向が正)と太さ。フォントが`post`テーブルを持たない場合は`None`。
    pub fn underline_metrics(&self) -> Option<(i16, i16)> {
        self.metrics.underline
    }

    /// 取り消し線の中心位置(ベースラインからの符号付きオフセット、フォントユニット。
    /// 上方向が正)と太さ。フォントが`OS/2`テーブルを持たない場合は`None`。
    pub fn strikeout_metrics(&self) -> Option<(i16, i16)> {
        self.metrics.strikeout
    }

    pub fn is_monospaced(&self) -> bool {
        self.metrics.is_monospaced
    }

    /// OS/2テーブルのウェイト値(400=標準, 700=太字)。
    pub fn weight(&self) -> u16 {
        self.metrics.weight
    }

    pub fn bounding_box(&self) -> ttf_parser::Rect {
        self.metrics.bounding_box
    }

    /// `glyph_id`の水平アドバンス幅(フォントユニット)。
    pub fn glyph_hor_advance(&self, glyph_id: u16) -> Option<u16> {
        self.face().glyph_hor_advance(ttf_parser::GlyphId(glyph_id))
    }

    /// `c`に対応するグリフをこのフォントが持っているか。
    /// font-familyフォールバック(どのフォントでこの文字を描画できるか)の判定に使う。
    /// 文字に対応するグリフID(cmapに無ければ`None`)。
    pub fn glyph_id(&self, c: char) -> Option<u16> {
        if let Some(cached) = self.glyphs.borrow().get(&c) {
            return *cached;
        }
        let found = self.face().glyph_index(c).map(|id| id.0);
        self.glyphs.borrow_mut().insert(c, found);
        found
    }

    pub fn has_glyph(&self, c: char) -> bool {
        self.glyph_id(c).is_some()
    }

    /// フォント名(`name`テーブルの Typographic Family、無ければ Family)。
    /// Unicodeエンコードの英語名のみ対応する。
    pub fn family_name(&self) -> Option<String> {
        self.metrics.family_name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

    #[test]
    fn loads_a_valid_font_file() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert!(font.face().units_per_em() > 0);
    }

    #[test]
    fn load_fails_for_missing_file() {
        let result = Font::load("/nonexistent/path/does-not-exist.ttf");
        assert!(result.is_err());
    }

    #[test]
    fn reports_family_name() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert_eq!(font.family_name().as_deref(), Some("DejaVu Sans"));
    }

    #[test]
    fn has_glyph_distinguishes_covered_and_uncovered_characters() {
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        assert!(font.has_glyph('A'));
        // DejaVu SansはCJK文字を含まない。
        assert!(!font.has_glyph('日'));
    }

    #[test]
    fn from_bytes_rejects_invalid_font_data() {
        let result = Font::from_bytes(b"not a font file".to_vec(), 0);
        assert!(result.is_err());
    }

    #[test]
    fn baseline_offset_is_between_zero_and_the_line_height() {
        // アセント分だけ行の上端から下がった位置が概ねベースラインになるはず
        // (行の高さがフォント自身のメトリクス通りなら半行送りはゼロに近い)。
        let font = Font::load(TEST_FONT_PATH).expect("should load bundled test font");
        let units_per_em = font.units_per_em() as f32;
        let ascent = font.ascender() as f32 / units_per_em * 16.0;
        let descent = -(font.descender() as f32) / units_per_em * 16.0;
        let line_height = ascent + descent;

        let offset = font.baseline_offset(16.0, line_height);
        assert!(
            (offset - ascent).abs() < 0.01,
            "with no extra leading, the baseline offset should equal the ascent: {offset} vs {ascent}"
        );
        assert!(offset > 0.0 && offset < line_height);
    }
}
