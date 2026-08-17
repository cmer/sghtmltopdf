//! SVGバイト列をPDFのForm XObjectへ変換する(ラスタライズしない)。
//!
//! パースとSVGの正規化(`use`の展開・スタイルの解決・座標系の畳み込み)は
//! usvg、そこからPDFのコンテンツストリームへの翻訳はsvg2pdfが行う。
//! どちらもtypst由来で、svg2pdfは`pdf_writer::Chunk`(このクレートが
//! 文書全体の書き出しに使っているのと同じ型)を返すため、変換結果は
//! バイト列を経由せずそのまま文書へ差し込める。
//!
//! # ラスタとの違い
//!
//! ラスタ画像は1枚が1つのImage XObjectになるが、SVGは「Form XObject 1つ +
//! それが参照するグラデーション・`ExtGState`・入れ子のXObject」という
//! **複数オブジェクトのかたまり**になる。そのため`Ref`の払い出しも
//! 「1〜2個」ではなく「チャンクに含まれるオブジェクトの数だけ」必要で、
//! [`renumber_into_document`]がそこを引き受ける。
//!
//! 描画側から見た使い勝手はラスタと同じで、svg2pdfが作るForm XObjectは
//! `/Matrix`で1x1の単位正方形へ正規化されている(Image XObjectと同じ)。
//! つまり`document::render_image`の`cm`(`[w, 0, 0, h, x, y]`)がそのまま効く。
//!
//! フォーマットの嗅ぎ分け([`looks_like_svg`])は`svg` featureが無くても
//! 使える。featureが無いときは「SVGだと分かった上で描けない」と言えた方が
//! 「対応していないフォーマット」より分かりやすいため。
//!
//! # フォント
//!
//! SVG内の`<text>`に使えるフォントは[`SvgFontDb`]が決める。**文書が使うのと
//! 同じ`FontCollection`から組む**ので、`--font`で渡したフォントも
//! `@font-face`で読み込んだフォントも、そのままSVGの中から引ける。
//! usvgに自前でシステムフォントを探させることはしない(この処理系の
//! フォント解決を二重に走らせないため)。

/// SVG内の`<text>`を描くためのフォントデータベース。
///
/// `svg-text` featureが無いときは中身を持たない値になり、SVG内のテキストは
/// 描画されない(パス化もされない)。
///
/// `svg-text`が有効なときは、文書の[`FontCollection`](crate::fonts::FontCollection)
/// にあるフォントのバイト列をそのまま持つ。usvgの`fontdb`は本体が使う
/// `fontdb`とは別バージョンの別インスタンスだが、**中身のフォントは同じもの**
/// になる。`Arc`で持つので複製は安い。
#[derive(Clone, Default)]
pub struct SvgFontDb {
    #[cfg(feature = "svg-text")]
    db: std::sync::Arc<svg2pdf::usvg::fontdb::Database>,
    /// `font-family`を持たない`<text>`に使う既定のfamily名。文書の先頭の
    /// フォント(=`--font`で最初に渡したもの)の名前を使う。
    #[cfg(feature = "svg-text")]
    default_family: Option<String>,
}

impl SvgFontDb {
    /// フォントを持たないデータベース。SVG内のテキストは描画されない。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 文書のフォントコレクションから組む。
    #[cfg(feature = "svg-text")]
    pub fn from_collection(fonts: &crate::fonts::FontCollection) -> Self {
        use svg2pdf::usvg::fontdb;

        let mut db = fontdb::Database::new();
        let mut default_family = None;
        for (index, font) in fonts.fonts().iter().enumerate() {
            // フォントのバイト列はそのまま渡す(ファイルを読み直さない。
            // `@font-face`の`data:`URIやHTTP取得のようにファイルが存在しない
            // 経路もあるため)。TTCのような複数フェイスのファイルでは
            // `load_font_source`が全フェイスを登録するので、文書が使う
            // フェイス番号(`Font::face_index`)以外も入る。SVG側の照合は
            // family名で行われるため、それで困ることはない。
            let ids = db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
                font.data().to_vec(),
            )));

            // CSS上で名乗っている名前(`@font-face`の`font-family`)が
            // フォント内部の`name`テーブルと違う場合、そのままではSVGから
            // 引けない。宣言名を別名として足しておく。
            if let Some(declared) = fonts.declared_family(index) {
                for id in ids {
                    let Some(mut info) = db.face(id).cloned() else {
                        continue;
                    };
                    if info
                        .families
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case(declared))
                    {
                        continue;
                    }
                    info.families
                        .push((declared.to_string(), fontdb::Language::English_UnitedStates));
                    db.remove_face(id);
                    db.push_face_info(info);
                }
            }

            if default_family.is_none() {
                default_family = fonts
                    .declared_family(index)
                    .map(str::to_string)
                    .or_else(|| font.family_name());
            }
        }

        Self {
            db: std::sync::Arc::new(db),
            default_family,
        }
    }

    /// `svg-text`が無効なときは、コレクションを見ずに空のまま返す
    /// (SVG内のテキストは描画されない)。
    #[cfg(not(feature = "svg-text"))]
    pub fn from_collection(_fonts: &crate::fonts::FontCollection) -> Self {
        Self::default()
    }

    /// 登録されているフェイスの数(テストと診断用)。`svg-text`が無効なら常に0。
    pub fn len(&self) -> usize {
        #[cfg(feature = "svg-text")]
        {
            self.db.len()
        }
        #[cfg(not(feature = "svg-text"))]
        {
            0
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for SvgFontDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgFontDb")
            .field("faces", &self.len())
            .finish()
    }
}

/// バイト列がSVG(またはgzip圧縮されたsvgz)に見えるか。
///
/// PNG/JPEG/WebPと違ってマジックバイトが無いため、XMLとして始まっていること
/// と、先頭付近に`<svg`が現れることの2点で判定する。ラスタのマジックバイト
/// 判定を先に通した後で呼ばれる前提。
pub fn looks_like_svg(bytes: &[u8]) -> bool {
    // svgz(gzip)。中身の判定はusvgのデコードに任せる。
    if bytes.starts_with(&[0x1F, 0x8B]) {
        return true;
    }

    // UTF-8 BOMと先頭の空白を読み飛ばす。
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let rest = match rest.iter().position(|b| !b.is_ascii_whitespace()) {
        Some(i) => &rest[i..],
        None => return false,
    };
    if !rest.starts_with(b"<") {
        return false;
    }

    // `<?xml ...?>`・コメント・DOCTYPEが前に付くので、ルート要素が先頭に
    // あるとは限らない。先頭の一定量だけを見て`<svg`を探す
    // (SVG以外のXMLを取り違えないよう、文書全体は走査しない)。
    const SNIFF_WINDOW: usize = 4096;
    let window = &rest[..rest.len().min(SNIFF_WINDOW)];
    window.windows(4).any(|w| w.eq_ignore_ascii_case(b"<svg"))
}

#[cfg(feature = "svg")]
mod convert {
    use std::collections::HashMap;

    use pdf_writer::{Chunk, Ref};

    use super::SvgFontDb;
    use crate::pdf::document::RefAllocator;

    /// 変換済みのSVG。1つの`<img src="*.svg">`(または`background-image`)に
    /// 対応する。
    ///
    /// この時点の`Ref`はsvg2pdfが1から振ったチャンク内ローカルな番号で、
    /// 文書の`Ref`空間とは無関係。実際に埋め込むときに
    /// [`renumber_into_document`]で振り直す。
    #[derive(Debug, Clone)]
    pub struct VectorGraphic {
        chunk: Chunk,
        /// `chunk`内のForm XObjectのRef(コンテンツストリームから`Do`する対象)。
        root: Ref,
    }

    /// 文書の`Ref`空間へ振り直したSVG。
    #[derive(Debug, Clone)]
    pub struct RenumberedVectorGraphic {
        pub chunk: Chunk,
        pub root: Ref,
        /// `chunk`内での各オブジェクトの開始オフセット。ストリーミング書き出しが
        /// xrefを組むのに使う(バッチ書き出しは`Chunk::extend`が自前で持つ
        /// オフセットを使うので要らない)。
        pub offsets: Vec<(Ref, usize)>,
    }

    /// SVGの変換に失敗した理由。
    #[derive(Debug)]
    pub struct SvgError(String);

    impl std::fmt::Display for SvgError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "SVGの変換に失敗しました: {}", self.0)
        }
    }

    impl std::error::Error for SvgError {}

    /// SVGバイト列をPDFのForm XObjectへ変換する。返り値の`width`/`height`は
    /// SVGの内在サイズ(px)で、ラスタ画像の`PreparedImage`と同じ意味を持つ。
    ///
    /// `fonts`はSVG内の`<text>`に使うフォント([`SvgFontDb`])。文書が使うのと
    /// 同じフォントを渡すことで、HTML側で使えるフォントがSVGの中でも使える。
    pub fn convert_svg(
        bytes: &[u8],
        fonts: &SvgFontDb,
    ) -> Result<(u32, u32, VectorGraphic), SvgError> {
        let options = svg_options(fonts);
        let tree =
            svg2pdf::usvg::Tree::from_data(bytes, &options).map_err(|e| SvgError(e.to_string()))?;

        let size = tree.size();
        if !size.width().is_finite() || !size.height().is_finite() {
            return Err(SvgError(format!(
                "内在サイズが数値になりません({}x{})",
                size.width(),
                size.height()
            )));
        }

        let conversion = svg2pdf::ConversionOptions {
            // SVGのコンテンツストリームはパスデータの羅列(テキスト)なので
            // 圧縮の効きが大きい。文書全体の`--no-compression`はPDFの構造を
            // 目で追うためのオプションで、埋め込んだSVGの中まで読めるように
            // する意図は無いため、ここは常に圧縮する。
            compress: true,
            ..Default::default()
        };
        let (chunk, root) =
            svg2pdf::to_chunk(&tree, conversion).map_err(|e| SvgError(e.to_string()))?;

        // 内在サイズは整数へ丸める(レイアウト側の内在サイズがu32のため)。
        // 小数のviewBox(`24.5`等)ではアスペクト比が僅かにずれるが、
        // `width`/`height`/`aspect-ratio`のいずれかが指定されていれば影響しない。
        let width = size.width().round().max(1.0) as u32;
        let height = size.height().round().max(1.0) as u32;
        Ok((width, height, VectorGraphic { chunk, root }))
    }

    /// usvgのパースオプション。
    ///
    /// # SVG内からのファイル読み出しを塞ぐ
    ///
    /// usvgの既定の`ImageHrefResolver::resolve_string`は、SVG内の
    /// `<image href="...">`を**そのまま`std::fs::read`する**
    /// (`usvg::parser::image`。usvgがライブラリとしてファイルに触る唯一の
    /// 箇所)。これは`img::fetch`が持っている封じ込め(基準ディレクトリの
    /// 外へ出る参照の拒否・`--allow`・`--disable-local-file-access`)を
    /// まるごと迂回する。
    ///
    /// 現状svg2pdfの`image` featureを切っているので、読まれた中身がPDFへ
    /// 出ることはない(`<image>`ノードは描画されずに捨てられる)。それでも
    /// 塞ぐのは、
    ///
    /// * 読み込み自体が起きる(ファイルの存在判定、上限無しの`fs::read`)
    /// * `image` featureを足した瞬間に「任意のファイルをPDFへ流し込める」
    ///   に変わる。入れ子のSVGはベクタのまま描画されるので、テキストや
    ///   図形がそのまま出る
    ///
    /// の2点のため。`resolve_string`を何も解決しない関数へ差し替えると、
    /// SVGの中から外部リソースを引く経路が無くなる。`data:`URIを扱う
    /// `resolve_data`は既定のまま残す。取得済みのバイト列の中で完結していて、
    /// 新たに信頼境界を越えないため。
    ///
    /// # フォント
    ///
    /// `resources_dir`は既定の`None`のまま(相対パスの解決先を与えない)。
    /// フォントは`fonts`から受け取ったものだけを使い、usvgに
    /// `load_system_fonts()`はさせない。この処理系はシステムフォントの探索を
    /// 自前で持っている(`fonts::system`)ので、二重に走らせる意味が無く、
    /// また「HTMLで使えるフォントがSVGでも使える」という対応が崩れるため。
    fn svg_options(fonts: &SvgFontDb) -> svg2pdf::usvg::Options<'static> {
        #[allow(unused_mut)]
        let mut options = svg2pdf::usvg::Options {
            image_href_resolver: svg2pdf::usvg::ImageHrefResolver {
                resolve_string: Box::new(|href, _| {
                    eprintln!(
                        "警告: SVG内の外部参照は読み込みません(SVGの中からは\n  \
                         ファイルを開けないようにしています): {href}"
                    );
                    None
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        #[cfg(feature = "svg-text")]
        {
            options.fontdb = fonts.db.clone();
            // `font-family`を持たない`<text>`はusvgの既定("Times New Roman")
            // ではなく文書の既定フォントで描く。手元に無いフォント名を既定に
            // 置くと、指定の無いテキストが必ず描けなくなる。
            if let Some(family) = &fonts.default_family {
                options.font_family = family.clone();
            }
        }
        #[cfg(not(feature = "svg-text"))]
        let _ = fonts;
        options
    }

    /// svg2pdfのチャンク内ローカルな`Ref`を、`alloc`から払い出した文書の`Ref`へ
    /// 振り直す。
    ///
    /// チャンクに含まれるオブジェクトの数だけ`Ref`を消費する。ストリーミング
    /// 書き出しのxrefは「1から連番で全て書き出されている」ことを前提にして
    /// いるので、**払い出した`Ref`と実際に書き出されるオブジェクトが1対1で
    /// 対応していること**を確認し、崩れていればエラーにする(壊れたPDFを
    /// 書くよりSVG1枚を落とす方が良い)。
    ///
    /// 失敗したときは`alloc`を進めない。番号だけ消費して書き出されない
    /// オブジェクトが残ると、SVGを落としたつもりで文書全体を壊してしまう。
    pub fn renumber_into_document(
        graphic: &VectorGraphic,
        alloc: &mut RefAllocator,
    ) -> Result<RenumberedVectorGraphic, SvgError> {
        let mut next = alloc.peek().get();
        let mut mapping: HashMap<Ref, Ref> = HashMap::new();
        let chunk = graphic.chunk.renumber(|old| {
            *mapping.entry(old).or_insert_with(|| {
                let assigned = Ref::new(next);
                next += 1;
                assigned
            })
        });

        let root = *mapping
            .get(&graphic.root)
            .ok_or_else(|| SvgError("Form XObjectのRefが振り直されませんでした".to_string()))?;

        let refs: Vec<Ref> = chunk.refs().collect();
        if refs.len() != mapping.len() {
            // チャンク内で参照されているのに定義されていないオブジェクトがある
            // (`Chunk::renumber`はそういう参照にもmappingを呼ぶ)。番号だけ
            // 消費して書き出されないオブジェクトが出るため受け付けない。
            return Err(SvgError(format!(
                "チャンク内に未定義の参照があります(定義{}件に対し参照{}件)",
                refs.len(),
                mapping.len()
            )));
        }

        let offsets = object_offsets(&chunk, &refs)?;
        alloc.commit(mapping.len());
        Ok(RenumberedVectorGraphic {
            chunk,
            root,
            offsets,
        })
    }

    /// `chunk`のバイト列中での各オブジェクトの開始位置を求める。
    ///
    /// `Chunk`はオフセットを内部に持っているが公開していないため、
    /// `Chunk::renumber`が書く並び(`{id} {gen} obj\n...\nendobj\n\n`が
    /// `refs()`の順に隙間なく続く)を前提に走査する。オブジェクトのヘッダは
    /// 直前の`endobj`に固定して探すので、ストリームの中身が偶然ヘッダらしく
    /// 見えても拾わない。1つでも見つからなければエラーを返す
    /// (推測でxrefを作るとPDF全体が壊れるため)。
    fn object_offsets(chunk: &Chunk, refs: &[Ref]) -> Result<Vec<(Ref, usize)>, SvgError> {
        const ENDOBJ: &[u8] = b"\nendobj\n\n";
        let bytes = chunk.as_bytes();
        let mut offsets = Vec::with_capacity(refs.len());
        let mut pos = 0usize;

        for (i, &id) in refs.iter().enumerate() {
            let header = format!("{} 0 obj\n", id.get()).into_bytes();
            let start = if i == 0 {
                // 最初のオブジェクトはチャンクの先頭にある。
                bytes.starts_with(&header).then_some(0)
            } else {
                // 2つ目以降は直前のオブジェクトの`endobj`に続く。
                let anchored = [ENDOBJ, &header].concat();
                find(&bytes[pos..], &anchored).map(|at| pos + at + ENDOBJ.len())
            };
            let start = start.ok_or_else(|| {
                SvgError(format!(
                    "オブジェクト{}({}番目)の開始位置が見つかりません",
                    id.get(),
                    i + 1
                ))
            })?;
            offsets.push((id, start));
            pos = start + header.len();
        }

        Ok(offsets)
    }

    /// `needle`が最初に現れる位置。
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const CIRCLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
            <circle cx="10" cy="5" r="4" fill="#f00"/>
        </svg>"##;

        #[test]
        fn converts_an_svg_and_reports_its_intrinsic_size() {
            let (width, height, graphic) =
                convert_svg(CIRCLE.as_bytes(), &SvgFontDb::empty()).expect("should convert");
            assert_eq!((width, height), (20, 10));
            assert!(
                graphic.chunk.refs().len() >= 1,
                "the chunk should hold at least the form XObject"
            );
        }

        #[test]
        fn rejects_bytes_that_are_not_svg() {
            assert!(convert_svg(b"not an svg at all", &SvgFontDb::empty()).is_err());
        }

        #[test]
        fn renumbering_maps_every_object_into_the_documents_ref_space() {
            let (.., graphic) = convert_svg(CIRCLE.as_bytes(), &SvgFontDb::empty()).unwrap();
            let mut alloc = RefAllocator::default();
            // SVGより先に何かを払い出しておき、1始まりに戻らないことを確かめる。
            let first = alloc.next();
            let renumbered = renumber_into_document(&graphic, &mut alloc).expect("should renumber");

            let mut got: Vec<i32> = renumbered.chunk.refs().map(|r| r.get()).collect();
            assert!(!got.is_empty());
            assert!(
                renumbered.chunk.refs().any(|r| r == renumbered.root),
                "the form XObject must be one of the chunk's objects"
            );
            // 払い出した`Ref`とチャンクのオブジェクトが1対1(xrefに穴が無い)。
            let expected: Vec<i32> = (first.get() + 1..=first.get() + got.len() as i32).collect();
            got.sort_unstable();
            assert_eq!(got, expected);
        }

        #[test]
        fn object_offsets_point_at_every_object_header() {
            let (.., graphic) = convert_svg(CIRCLE.as_bytes(), &SvgFontDb::empty()).unwrap();
            let mut alloc = RefAllocator::default();
            let renumbered = renumber_into_document(&graphic, &mut alloc).unwrap();

            let bytes = renumbered.chunk.as_bytes();
            assert_eq!(renumbered.offsets.len(), renumbered.chunk.refs().len());
            for &(id, offset) in &renumbered.offsets {
                let header = format!("{} 0 obj\n", id.get());
                assert!(
                    bytes[offset..].starts_with(header.as_bytes()),
                    "offset {offset} for object {} should point at its header",
                    id.get()
                );
            }
        }
    }
}

#[cfg(feature = "svg")]
pub use convert::{convert_svg, renumber_into_document, RenumberedVectorGraphic, VectorGraphic};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_a_plain_svg() {
        let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"></svg>"#;
        assert!(looks_like_svg(src.as_bytes()));
    }

    #[test]
    fn sniffs_an_svg_behind_a_prolog_a_comment_and_a_doctype() {
        let src = concat!(
            "\n  <?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!-- exported by some drawing tool -->\n",
            "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x.dtd\">\n",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
        );
        assert!(looks_like_svg(src.as_bytes()));
    }

    #[test]
    fn sniffs_an_svg_with_a_utf8_bom_and_an_uppercase_tag() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<SVG xmlns=\"http://www.w3.org/2000/svg\"></SVG>");
        assert!(looks_like_svg(&bytes));
    }

    #[test]
    fn does_not_sniff_other_xml_or_binary_as_svg() {
        assert!(!looks_like_svg(
            b"<?xml version=\"1.0\"?><rss><channel/></rss>"
        ));
        assert!(!looks_like_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_svg(b""));
        assert!(!looks_like_svg(b"   \n\t  "));
        // `<svg`が遠くにあるだけのXMLは拾わない。
        let mut far = String::from("<other>");
        far.push_str(&"x".repeat(5000));
        far.push_str("<svg/></other>");
        assert!(!looks_like_svg(far.as_bytes()));
    }
}
