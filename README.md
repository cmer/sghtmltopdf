# sghtmltopdf

Chromium/WebKit/Geckoに依存しない軽量なHTML→PDFレンダラー。
wkhtmltopdfに敬意を表して、Second Generationを付けてライブラリ名は **sghtmltopdf**。

> **Note**: 本プロジェクトはhtml5ever/Stylo/Taffy/KrillaなどServo/Rustエコシステム由来のクレートを利用していますが、
> Servo公式プロジェクトとは無関係の独立した実装です。

wkhtmltopdf(2023年1月アーカイブ済み)の後継として、headless Chrome方式(Puppeteer/Playwright等)が抱える
CPU負荷・webfont待機・メモリ使用量・サーバレス環境での制約といった課題の解決を目指します。

## リポジトリ構成

```
.
├── core/           # Rustコア(Ruby非依存、CLIとしても動作)
└── bindings/
    └── ruby/       # Ruby向けFFI層(magnus + rb-sys)
```

## CSSプロパティ対応を追加する際の開発手順

M8(CSS2.1対応)の各カテゴリ(Positioning & Float、Typography詳細、Table
layout、Lists、Box model詳細、Generated content、Background詳細)を実装する
過程で固まった、CSSプロパティ対応を1カテゴリ追加する際の標準的な手順。
新しいカテゴリ・M9(CSS3対応)以降もこの手順を踏襲する想定。

1. **現状調査**: `core/src/style/properties.rs`(`PropertyDeclaration`)・
   `computed.rs`(`ComputedStyle`)を対象カテゴリのプロパティでgrepし、
   パース・カスケードの対応状況を事実ベースで確認する(投機で書かない)。
2. **スコープ判断の洗い出し**: 仕様のどこまで対応するか(キーワードの
   組み合わせ、非対応にするCSS3寄りの値、印刷用途で意味を持たない機能等)
   のうち、コードだけでは決められない判断はユーザーに確認する。判断は
   `docs/decisions/NNNN-{category}-design.md`(連番ADR)に、決定した内容と
   理由をセットで記録する(対応表そのものはADRには置かず、
   `docs/tasks/000N-*.md`側のカテゴリ別✅/❌表で管理する。カテゴリ単位・
   用途優先の進め方の背景は
   [docs/decisions/0018-css21-css3-coverage-strategy.md](docs/decisions/0018-css21-css3-coverage-strategy.md)参照)。
3. **値の型を`style/values.rs`に追加**: 長さ・パーセンテージを持つ値は、
   パース直後の指定値(`Specified*`、`em`/`rem`等の相対単位を保持)と
   カスケード解決後の計算値(`resolve(font_size, root_font_size)`でpxへ
   変換)を分離する既存パターン(`SpecifiedCornerRadius`/`CornerRadius`等)
   に倣う。単純なキーワードのみの値は指定値/計算値を分けない1つのenumで足りる。
4. **パースを`style/properties.rs`に追加**: `PropertyDeclaration`へ
   バリアントを追加し、`parse_declaration`のディスパッチ表とパース関数を
   実装する。ショートハンドは「ループで、どの種類の値かを`try_parse`で
   都度判定する」既存パターン(`border`/`outline`/`list-style`ショートハンド
   参照)を踏襲する。
5. **カスケードを`style/computed.rs`に追加**: `ComputedStyle`へフィールドを
   追加し、継承プロパティかどうかを決めた上でカスケード収集ループ・
   継承解決・初期値フォールバックを実装する。
6. **レイアウト/PDF描画を実装**: 必要に応じて`layout/`配下(ボックスツリー
   構築・レイアウトアルゴリズム)と`pdf/document.rs`(実際の描画)を拡張する。
   幾何計算は描画コードから独立した純粋関数に切り出すと単体テストしやすい
   (`pdf/document.rs::background_tile_rects`等)。
7. **テストを追加**: `computed.rs`にパース・継承・カスケードの単体テスト、
   `pdf/document.rs`に描画ロジックの単体テストを追加した上で、
   `core/tests/{category}.rs`を新設し、HTMLパース→スタイルカスケード→
   ページ分割→PDFエンコードの実際のパイプラインを通すE2Eテストを書く。
8. **CLIで目視確認**: `cargo run --bin sghtmltopdf`で対象プロパティを含む
   HTMLを実際にPDF化し、PyMuPDF等でPNGへレンダリングして目視確認する
   (自動テストは正しさの一部しか検証できないため必須の手順とする)。
9. **lintとテストをクリーンにする**: `cargo fmt --check`・
   `cargo clippy --workspace --all-targets -- -D warnings`・
   `cargo test --workspace`がすべて通ることを確認する。
10. **ドキュメントを同期する**: `docs/tasks/000N-*.md`のカテゴリ別対応表を
    ✅へ更新しT番号タスクとして実装内容を追記、README.mdの「開発状況」に
    完了したカテゴリの節を追加する(実装完了分にのみ適用する新規ルールで、
    過去分の遡及は行わない)。
11. **コミットは人間が行う**。実装が完了したら、コミット内容の提案(対象
    ファイル・コミットメッセージ案)を投げる。

## 開発状況

マイルストーン1(静的HTML一括変換)・マイルストーン2(CSS Fragmentation)・
マイルストーン3(ストリーミング入出力対応)・マイルストーン4(Webfont対応)・
マイルストーン5(画像埋め込み)・マイルストーン6(外部スタイルシート)・
マイルストーン7(`@import`/`background-image`のurl()対応)
ともに完了。マイルストーン8(CSS2.1対応)は、Phase 1(Positioning & Float・
Typography詳細・Table layout完全対応・Lists)・Phase 2(Box model詳細・
Generated content・Background詳細)ともに完了し、**マイルストーン8全体が完了**。
**マイルストーン9(CSS3対応)全体が完了**(Phase 1: `box-sizing`・Paged media、
Phase 2: Color Level 4・`object-fit`/`object-position`・`box-shadow`・
CSS Custom Properties、Phase 3: Flexbox)。M9完了後、ユーザー指示により
Phase4で対象外候補としていた`opacity`/`transform`にも対応した(`filter`は
引き続き対象外)。

### `opacity`/`transform`

```css
.watermark { opacity: 0.4; transform: rotate(-20deg); }
.card { transform: scale(1.1); transform-origin: top left; }
```

* `transform`は`translate`/`translateX`/`translateY`/`scale`/`scaleX`/
  `scaleY`/`rotate`/`skew`/`skewX`/`skewY`/`matrix()`に対応(3D変形・
  `perspective`は非対応)。複数関数は記述順に合成する。`transform-origin`
  (初期値`50% 50%`)にも対応。レイアウトには一切影響しない視覚効果のみ
  (CSS仕様通り)で、PDFコンテンツストリームの`cm`(CTM)操作だけで実装して
  いる
* `opacity`はPDFの透明グループ(Form XObject + `/Group /S /Transparency`)を
  使った、CSS仕様通りの正確な実装。要素のサブツリー全体(背景・枠線・
  テキスト・子要素すべて)を1枚の絵として下地に対して1回だけ半透明合成する
  ため、サブツリー内で子要素同士が重なっていても二重に暗く合成されない
  (単純に各描画命令へ同じアルファを個別適用する近似とは異なる)
* `opacity`と`transform`を同じ要素に指定した場合、`transform`のCTMが
  `opacity`の透明グループ呼び出しを内包する(変形してから半透明合成)
* ページ分割・ストリーミング出力の両方で動作する

詳細は[docs/decisions/0035-opacity-transform-design.md](docs/decisions/0035-opacity-transform-design.md)
参照。

### Flexbox(`display: flex`)

```css
.invoice-header { display: flex; justify-content: space-between; align-items: center; }
.item-row { display: flex; }
.item-row .name { flex-grow: 1; }
.item-row .price { flex-shrink: 0; width: 80px; }
```

* `flex-direction`/`flex-wrap`/`justify-content`/`align-items`/
  `align-content`/`gap`(コンテナ側)、`flex-grow`/`flex-shrink`/
  `flex-basis`/`flex`ショートハンド/`align-self`(アイテム側)に対応
* レイアウトは`taffy`クレートへブリッジする形で実装している。既存のbox tree
  (`display: table`と同じパターン)のサブツリーとしてflexコンテナを扱い、
  テキスト等の内在サイズが必要な採寸は既存のブロック/インライン/テーブル
  レイアウト関数を呼んで実測する(2パス方式)
* `inline-flex`は非対応。`order`もtaffy自体が未対応のため非対応
* min-contentとmax-contentは区別しない(既知の簡略化)
* flexコンテナはページ分割に対してアトミック(`display: table`と同じく、
  内部で分割せず丸ごと次ページへ送る)
* Grid非対応(必要になれば別途検討)

詳細は[docs/decisions/0034-flexbox-design.md](docs/decisions/0034-flexbox-design.md)
参照。

### CSS Custom Properties(`--foo`/`var()`)

```css
:root { --brand-color: rgb(20, 90, 200); --gap: 24px; }
.card {
  width: var(--brand-width, 300px);
  padding: var(--gap);
  background-color: var(--brand-color);
}
```

* 仕様通りのcascade/継承ベースの実装ではなく、`@import`展開(`resolve_imports`)
  と同じ「パース前のテキスト置換」で実装している(目的はテンプレート側の
  保守性向上であり、JS実行非対応の本プロジェクトでは要素ごとに動的に変わる
  値はそもそも起こらないため)
* カスタムプロパティは文書全体でフラットな名前空間になる。セレクタの詳細度・
  オリジンは見ず、`<style>`/`<link>`を連結したテキスト上での出現順で
  最後に定義されたものが文書全体で使われる
* `var(--foo, fallback)`のフォールバックに対応。名前未定義かつフォールバック
  も無い場合は`var(...)`をそのまま残し、後段のプロパティパーサが未知トークン
  として黙って無視する既存の経路に乗る
* `style="..."`インライン属性内の`var()`は非対応(既知の簡略化)

詳細は[docs/decisions/0033-css-custom-properties-design.md](docs/decisions/0033-css-custom-properties-design.md)
参照。

### `object-fit`/`object-position`

`<img>`版の`background-size: cover/contain`に対応している。

```css
img { width: 150px; height: 80px; object-fit: cover; object-position: center; }
```

* `object-fit`は`fill`(初期値、非一様に引き伸ばす)/`contain`/`cover`/
  `none`/`scale-down`に対応
* `object-position`は`background-position`と同じ値の文法(初期値は
  `50% 50%`、`background-position`の`0% 0%`とは異なる)
* `object-fit`の値によらず、常にcontent-boxへクリップして描画する
  (`fill`は元々ぴったり収まるがno-opとして同じ経路を通る)

詳細は[docs/decisions/0030-object-fit-position-design.md](docs/decisions/0030-object-fit-position-design.md)
参照。

### `box-shadow`

カンマ区切りの複数指定・ぼかし(`blur-radius`)・広がり(`spread-radius`)に
対応している。

```css
.card { box-shadow: 0 4px 8px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(0, 0, 0, 0.1); }
```

* `inset`はパースするが描画は非対応(既知の簡略化。外側の影のみ実務上の
  需要が高いと判断)
* ぼかしは真のガウスぼかしではなく、4段階の同心半透明矩形を重ね塗りして
  近似する(`border-style`のgroove/ridge/inset/outsetの疑似陰影と同じ
  判断基準)
* リスト内の複数指定は、先頭が最前面になる(仕様通り)
* この実装の前提として、`background-color: rgba(...)`等の半透明色が実際には
  不透明として描画される既存の欠陥を発見・修正した(次節参照)

詳細は[docs/decisions/0032-box-shadow-design.md](docs/decisions/0032-box-shadow-design.md)
参照。

### 単色塗りのアルファ透過(ExtGState)

`background-color`/`box-shadow`の半透明色(`rgba()`等のアルファ値)が、
PDFの`ExtGState`(`/ca`)を使って正しく半透明に描画されるようになった。

* 0.05刻み・21段階のExtGStateを文書全体で1回だけ確保し、フォントと同じく
  全ページの`Resources`辞書へ無条件で列挙する(画像のような使用状況の
  動的収集は行わない)
* 対象は`background-color`と`box-shadow`の塗りに限定する。`border-color`/
  `outline-color`/テキストの`color`の半透明指定は今回のスコープ外で、
  引き続き不透明として描画される(既知の簡略化)

詳細は[docs/decisions/0031-fill-alpha-design.md](docs/decisions/0031-fill-alpha-design.md)
参照。

### Color Level 4(`lab()`/`lch()`/`oklab()`/`oklch()`)

CIE Lab/LCH・Oklab/Oklchの各色関数からsRGBへの変換に対応している。

```css
.box { background-color: oklch(59.686% 0.15619 49.7694deg); }
```

* 変換は`palette`クレート(`default-features = false, features = ["std"]`
  に絞り込み、実行時依存は`palette`本体と`fast-srgb8`のみ)に委ねている。
  自前で変換行列(CIE Lab→XYZ→linear sRGB、Oklab→linear sRGB)を実装する
  よりも、型安全な変換を低コストに使える判断とした
* `hsl()`/`hwb()`と同じ設計で、パース時点でsRGBへ変換し保持する
  (`Color`型にLab/LCH等の色空間情報を保持する新バリアントは追加しない)
* 各成分がCSS仕様のnominal range(`lab()`のL: 0〜100、`oklch()`のC: 0〜0.4
  等)を超える値は、`hsl()`/`hwb()`と同じくsRGBへの変換結果を0〜1へクランプ
  する
* 相対色構文(`lab(from ...)`)・`color-mix()`・`color()`(`display-p3`等の
  予測済み色空間)はCSS Color 5相当・スコープ外のため非対応

詳細は[docs/decisions/0029-color-level4-design.md](docs/decisions/0029-color-level4-design.md)
参照。

### Paged media(`@page`/`@media`/margin box/ページ番号)

`@page`ルール(`size`/`margin`、文書全体で1回だけ適用される単一の上書き)・
`@media`(`print`/`all`は常時適用、`screen`は常時無視、特徴クエリは評価しない)・
ページ余白ボックス(margin box、`@top-left`〜`@bottom-right-corner`の16個)・
`content: counter(page)`/`counter(pages)`(ページ番号)に対応している。

```css
@page {
  size: A4;
  margin: 80px 60px;
  @top-center { content: "請求書"; }
  @bottom-center { content: "Page " counter(page) " of " counter(pages); }
}
@page :first {
  @bottom-center { content: "表紙"; }
}
```

* `@page`の`size`/`margin`はページごとに変えられない(`:first`/`:left`/
  `:right`付きの`size`/`margin`宣言はパースされるが適用されない)。
  `:first`/`:left`/`:right`は**margin boxの内容の出し分け**にのみ使える
  (実装コストの大きい`paginate.rs`側の変更を避けるための意図的なスコープ
  縮小、ユーザー確認済み)
* margin boxの寸法は簡略化したモデルを使う: 4隅は固定サイズ(縦横マージン幅の
  交差部分)、残り12個(上/下/左/右の各辺3分割)は均等割り(著者の`width`指定は
  無視する)。装飾(背景・枠線)は非対応、`content`のテキスト描画のみ
* `counter(page)`(現在ページ番号)はストリーミングモードでも問題なく動作する。
  `counter(pages)`(総ページ数)は文書全体のページ分割が完了するまで値が
  確定しないため、**`Mode::Streaming`では非対応**(`EngineError::
  UnsupportedInStreamingMode`)。`Mode::Batch`では、`counter(pages)`が
  使われている場合のみ総ページ数を事前カウントするパスが1回余分に走る
* `@media`は特徴クエリ(`(min-width: ...)`等)を一切評価しない。sghtmltopdfは
  常に印刷/PDF出力のみでビューポートという概念が無いため

詳細は[docs/decisions/0028-paged-media-design.md](docs/decisions/0028-paged-media-design.md)
参照。

### `box-sizing`

`content-box`(既定)/`border-box`に対応している(`padding-box`は標準外の
ため非対応)。`border-box`の場合、指定した`width`/`height`はpadding・border
を含む外寸として扱われる(content-boxの実寸はそこからpadding+borderを
引いた値になり、それが0未満になる場合は0にクランプされる)。

* 非継承プロパティ(子要素には伝播しない)
* テーブルセルの列幅アルゴリズムには組み込んでいない(実務上`<td>`への
  明示指定は稀と判断、既知の簡略化)

詳細は[docs/decisions/0027-box-sizing-design.md](docs/decisions/0027-box-sizing-design.md)
参照。

### Generated content

`content`プロパティ(文字列・`attr()`・`counter()`/`counters()`)・CSSカウンタ
(`counter-reset`/`counter-increment`)・`::before`/`::after`/`::first-letter`
疑似要素・`quotes`(`open-quote`/`close-quote`)に対応している。

* `counter-reset`のスコープはCSS仕様通り、その要素自身と後続の兄弟要素まで
  有効で、親要素の子要素ループが完了した時点でようやくpopされる(そのpop
  タイミングを誤ると、兄弟間でカウンタ値が正しく引き継がれない)
* `::after`の`content`解決は子孫要素の処理が全て終わった後(DOM順で最後)に
  行う。子孫内の`counter()`増加や`quotes`のネスト深度変化を`::after`側の
  出力に反映するために必要
* `quotes`はカウンタと違いスコープを持たない単一のネスト深度カウンタで
  管理する(ネストした`<q>`ごとに深度に応じた引用符ペアを選ぶ)
* `::first-letter`はフォント・色・text-decoration・text-transformなど
  一部のプロパティのみ上書きできるスパースな上書き(float・box model等は
  非対応)。`::first-line`は非対応
* `::before`/`::after`/`content`はブロック子を持つ要素では非対応(簡略化)。
  例えば入れ子`<ol>`を含む`<li>`自身の`::before`は表示されない
  (入れ子`<ol>`側の`::before`は問題なく機能する)

詳細は[docs/decisions/0024-generated-content-design.md](docs/decisions/0024-generated-content-design.md)
参照。

### Box model詳細

`overflow`(hidden/scroll/auto)・`z-index`・`outline`(-color/-style/-width)・
`visibility`・`border-style`(groove/ridge/inset/outset)・`border-radius`楕円
(水平/垂直別半径)に対応している。

* `overflow`は`hidden`/`scroll`/`auto`を区別せず同じクリップ処理として扱う
  (印刷にスクロールの概念が無いため)。クリップ境界は常に直線のpadding-box
  (`border-radius`には沿わせない)
* `z-index`は同一の直接の親を持つ兄弟間の描画順のみを制御する(スタッキング
  コンテキストの分離は非対応)。`position: static`の要素には効果を持たない
  (仕様通り)
* `outline`はレイアウトに一切影響しない装飾として実装している(既存の
  `border`描画ロジックをborder-boxの外側で再利用)
* `visibility: hidden`(`collapse`も同一視)は、要素自身の描画は行わないが
  レイアウト上のスペースはそのまま占有する(`display: none`との違い)。
  子孫が`visibility: visible`で明示的に上書きしていれば正しく再表示される
* `border-style`のgroove/ridge/inset/outsetは、border-colorから算出した
  明暗2色(固定比率でのブレンド、正確な色再現は目指さない)で疑似立体陰影を
  描画する。`border-radius`との組み合わせは非対応(直線4辺にフォールバック)
* `border-radius`は各コーナーに独立した水平/垂直半径を持てるため、
  `border-radius: 60px / 30px`のような楕円コーナーに対応する

詳細は[docs/decisions/0023-box-model-details-design.md](docs/decisions/0023-box-model-details-design.md)
参照。

### Lists

`list-style-type`(disc/circle/square/decimal/decimal-leading-zero/
lower-roman/upper-roman/lower-alpha/upper-alpha/none)・
`list-style-position`(outside/inside)・`list-style`ショートハンドに対応している。

* マーカーの番号付けは、あるコンテナ(`<ul>`/`<ol>`等)の直接の子のうち
  `display: list-item`であるものを数えるローカルなカウンタで実現する。
  入れ子の`<ol>`/`<ul>`はそれぞれ独立したスコープを持つため1から数え直す
  (CSS3の汎用`counter-reset`/`counter-increment`機構は非対応)
* `<ol start="N">`のHTML属性に対応(`reversed`属性、`<ol type="...">`等の
  レガシーHTML属性は非対応)
* `list-style-position: outside`(初期値)はマーカーをcontent boxの外側
  (左のgutter)に独立して配置する。`inside`は`li`の内容がテキストのみの場合、
  `::before`と同じ要領で先頭のインラインコンテンツとして織り込む(ブロック子を
  持つ`li`では`outside`と同じ描画にフォールバックする)
* `list-style-image`はパースのみ対応し、実際には常に`list-style-type`の
  テキストマーカーへフォールバックする(画像マーカー自体は描画しない)

詳細は[docs/decisions/0022-list-style-design.md](docs/decisions/0022-list-style-design.md)
参照。

### Table layout完全対応

`rowspan`/`colspan`・`border-collapse`(collapse込み)・`border-spacing`・
`caption`/`caption-side`・`vertical-align`(テーブルセル文脈、top/middle/
bottom/baseline)・`table-layout`(auto/fixed)・`empty-cells`に対応している。

* `border-collapse: collapse`は列幅・セル配置の計算(レイアウト)自体は
  `separate`モデルと完全に同一に保ち、見た目の枠線描画のみを隣接セル間で
  統合する(幅→スタイルの優先順位で1本に統合、cellとtable自体の境界は対象外)
* `rowspan`はCSS2.1 17.2相当のテーブルグリッド構築(occupancy追跡)で対応し、
  複数行にまたがるセルの必要高さは跨ぐ行へ均等配分する(`colspan`の列幅不足分
  配ロジックと同型)
* `vertical-align: baseline`は、行内の各セルの「先頭行のベースライン位置」を
  求めて揃える。ベースラインを提供できないセル内容(ネストしたテーブル・
  置換要素)は`bottom`相当にフォールバックする
* `table-layout: fixed`は最初の行の明示`width`指定のみで列幅を決定し、内容の
  測定を完全にスキップする
* `<caption>`要素は`caption-side`(top/bottom)に応じて表の上下どちらかに配置
  される(従来`<caption>`の内容が完全に失われるバグがあったため、本対応で
  修正した)

詳細は[docs/decisions/0021-table-layout-design.md](docs/decisions/0021-table-layout-design.md)
参照。

### Typography詳細

`text-align`(left/right/center/justify)・`line-height`・`text-indent`・
`white-space`(normal/nowrap/pre)・`letter-spacing`・`word-spacing`・
`text-transform`(uppercase/lowercase/capitalize)に対応している。
`vertical-align`(インライン文脈)は非対応。

* `text-align: justify`は最後の行を除く各行の単語間ギャップを均等に伸縮する
  (CJK文字列内部には配分しない)
* `line-height`の`<number>`/`<percentage>`はCSS仕様通り未乗算のまま継承され、
  各テキストが自身のfont-sizeを基準に乗算する
* `text-indent`は最初の物理行のみに適用する(`white-space: pre`でも同様)
* `white-space: nowrap`は折返しを行わずオーバーフローを許容、`pre`は改行・
  連続空白をそのまま保持し折り返さない
* `letter-spacing`はPDFの`Tc`(character spacing)、`word-spacing`はレイアウト層の
  gap幅加算で実現する(`Tw`は複合フォントの2バイトコードに効かないため使わない)
* `text-align`/`text-indent`/`white-space`は同一インラインフォーマッティング
  コンテキスト内の先頭テキストノードの計算値で代表する(通常のブロック単位の
  指定であれば問題にならない、既知の簡略化)

詳細は[docs/decisions/0020-typography-details-design.md](docs/decisions/0020-typography-details-design.md)
参照。

### `float`/`clear`/`position: relative`

CSS2.1の`float`(left/right)・`clear`(left/right/both)、および
`position: relative`(top/right/bottom/leftによる視覚的オフセット)に対応している。
`position: absolute`/`fixed`は非対応。

* floatの周りのinlineコンテンツ(テキスト)は回り込む。floatの高さを過ぎた行は
  元の幅に復帰する
* 同方向の複数floatは横に並び、幅が足りなければ次の空きY位置に折り返す
  (CSS2.1 9.5.1の簡略版、厳密な最小隙間探索ではない)
* floatはブロックレベルの通常フローには寄与しない(後続のブロック要素はfloatの
  下を素通りして配置され、floatと重なり得る)。直接の子floatがコンテナの
  auto-heightより下に伸びていれば、その分だけ高さを拡張する(孫要素には伝播
  しない浅い実装)
* floatの`width: auto`はshrink-to-fit非対応。明示`width`を推奨(`<img>`のように
  内在サイズを持つ要素以外をfloatさせる場合)
* floatがページの残り高さに収まらない場合、複数ページに跨ることを許容する
* `position: relative`のオフセットは後続要素のフローに影響しない(視覚的位置の
  みずれる)。改ページ判定はオフセット適用後の座標で行う(既知の簡略化)

詳細は[docs/decisions/0019-float-clear-position-relative-design.md](docs/decisions/0019-float-clear-position-relative-design.md)
参照。

### 外部スタイルシート

`<head>`内の`<link rel="stylesheet" href="...">`によるCSSの外部読み込みに
対応している。ローカル相対パス(`base_dir`基準、先頭`/`のroot-relativeな
書き方も含む)・`http(s)`絶対URL・`data:` URIのいずれの`href`にも対応する
(画像埋め込みと同じ`--allow-remote-assets`フラグ・SSRF対策を共有する)。

* 複数の`<link>`・`<style>`が混在する場合、document中の出現順を保ったまま
  連結してからカスケードに反映する(CSSの「後勝ち」ルールがソース順通りに
  働く)
* 取得に失敗した外部スタイルシート(ネットワークエラー・SSRFブロック・
  非2xxステータス・不正なUTF-8、いずれも同列)は、そのスタイルシートだけを
  無視して警告を出し、文書生成全体は継続する(壊れた/ブロックされたURLで
  生成全体を止めない、画像埋め込みと同じ方針)
* フェッチした外部CSS自身に含まれる相対`url()`(`@font-face`の`src`等)は、
  そのスタイルシート自身の場所ではなく常に元HTMLの`base_dir`を基準に
  解決する(スタイルシートごとの基準切り替えは非対応の簡略化)。この
  制約の影響を受けないよう、外部スタイルシート内のフォント参照には
  root-relativeなパス(例: `url("/fonts/brand.woff2")`)を使うことを推奨する

詳細は[docs/decisions/0015-external-stylesheet-fetch-design.md](docs/decisions/0015-external-stylesheet-fetch-design.md)
参照。

### `@import`

`<style>`・外部スタイルシート(`<link>`)いずれの中に書かれた
`@import url(...)`にも対応している。パース前のCSSテキストに対する
展開処理として実装されており(`parse_stylesheet`自体は`@import`を
知らない)、`@import`文があった位置にそのままimport先の内容を差し込む
(hoistして先頭にまとめるのではない)。

* import先のCSSにさらに`@import`が含まれる多段importにも対応する。
  循環import(`a.css`が`b.css`をimportし`b.css`が`a.css`をimportする等)は
  再帰深さの上限(16階層)でガードし、無限再帰にはならない
* メディアクエリ付きの`@import url(...) screen;`は、`@media`自体が
  非対応スコープのため無条件importとして扱う
* import先の取得に失敗した場合(ネットワークエラー・SSRFブロック・
  非2xxステータス・不正なUTF-8)は、その`@import`文だけ無視して警告を出し、
  残りのCSSは正常にパースを継続する

詳細は[docs/decisions/0016-at-import-resolution-design.md](docs/decisions/0016-at-import-resolution-design.md)
参照。

### `background-image`と`background-position`/`-size`/`-repeat`/`-attachment`

`url(...)`によるCSSプロパティ値としての背景画像指定に加え、
`background-position`(キーワード+長さ・パーセンテージ、順序非依存)・
`background-size`(`cover`/`contain`込み)・`background-repeat`
(`repeat`/`repeat-x`/`repeat-y`/`no-repeat`)・`background-attachment`・
`background`ショートハンドに対応している(`<img>`の埋め込みと同じ
フェッチ層・SSRF対策・`--allow-remote-assets`フラグを共有する)。

* `background-size: cover`/`contain`は画像のintrinsicサイズ(デコード時に
  判明する実ピクセルサイズ)を基準にスケールを計算する
* `background-repeat`でのタイル敷き詰めはborder-boxへクリップして描画する。
  病的に小さい`background-size`が指定された場合に備え、1軸あたり200タイルで
  打ち切る防御的な上限を設けている
* `background-attachment: fixed`は`scroll`と同一視する(印刷/ページ
  ネーション文脈では「ビューポート固定」の概念自体が曖昧なため)
* `background`ショートハンドは、CSS仕様通り指定されなかったロングハンドを
  全て初期値へリセットする(`border`/`list-style`ショートハンドは以前の
  宣言を引きずる簡略化を採っているが、`background`は実務での使用頻度を
  踏まえ仕様に忠実にした)
* `border-radius`が指定されていても、背景画像は角丸にクリップされず
  常に直線の矩形として描画される(既知の簡略化)
* 取得・デコードに失敗した背景画像は、その要素だけ背景画像なし扱いにし、
  文書全体の生成は止めない(`<img>`と同じフォールバック方針)
* 背景色→背景画像→枠線の順で描画する

詳細は[docs/decisions/0017-background-image-design.md](docs/decisions/0017-background-image-design.md)・
[docs/decisions/0025-background-details-design.md](docs/decisions/0025-background-details-design.md)
参照。

### 画像埋め込み

`<img>`要素(JPEG/PNG/WebP)の埋め込みに対応している。ローカル相対パス
(`base_dir`基準)・`http(s)`絶対URL・`data:` URIのいずれの`src`にも対応する。

* JPEGはデコードせず、SOF0/SOF2マーカーからwidth/height/コンポーネント数
  だけを読んでDCTDecodeフィルタでそのまま埋め込む(再エンコードなし)。
  PNG/WebPはフルデコードし、アルファチャンネルがあれば`/SMask`付きの
  透過画像として埋め込む
* リモート画像フェッチは既定無効(CLIの`--allow-remote-assets`で
  オプトイン。外部スタイルシートのリモートフェッチと共通のフラグ)。
  有効化してもプライベート/loopback/link-local(クラウドメタデータの
  `169.254.169.254`含む)IP宛のリクエストは常にブロックする(SSRF対策。
  DNSリバインディング・リダイレクト経由のバイパスも同じ仕組みで防ぐ)
* `width`/`height`属性・CSSの`width`/`height`が無指定の場合、画像の内在
  サイズを使う。片方だけ指定されていればアスペクト比を保って他方を導出する
* 取得・デコードに失敗した画像はその要素だけ空扱いにし、文書全体の生成は
  止めない
* 同一文書内で同じ画像が繰り返し使われても、フェッチ・デコード・PDFへの
  埋め込みはいずれも初回の1回のみ(`Mode::Streaming`でもメモリ使用量は
  異なる画像の種類数に比例し、要素数には比例しない)
* `<img>`はブロックレベルの置換要素としてのみ対応する(インライン
  フォーマッティングコンテキストへの統合は非対応、既知の制約)

詳細は[docs/decisions/0012-image-embedding-crates.md](docs/decisions/0012-image-embedding-crates.md)
(クレート選定)・[docs/decisions/0013-image-fetch-security.md](docs/decisions/0013-image-fetch-security.md)
(SSRF対策)・[docs/decisions/0014-image-streaming-and-fallback.md](docs/decisions/0014-image-streaming-and-fallback.md)
(ストリーミング両立性・フォールバック)参照。

### Webfont対応

`@font-face`の`unicode-range`ディスクリプタに対応している。

* `unicode-range: U+0-24F, U+1E00-1EFF;`のような単一コードポイント/範囲/
  ワイルドカード(`U+4??`)/カンマ区切り複数レンジの構文をサポートする
* 宣言されたrangeはハードフィルタとして働く: range外の文字には、そのフォントが
  実際にグリフを持っていても使わない。range未指定のフォント(`local()`/`--font`/
  システムフォント自動探索を含む)は従来通り全域をカバーする(後方互換)
* 同一family名で複数の`@font-face`にunicode-rangeを分けて宣言する典型的な
  webfont配信パターン(英数字用フォント+CJK用フォントの併用等)に対応する。
  重複するrangeが同じ文字をカバーする場合は、CSSソース中で先に宣言された
  方を優先する

詳細は[docs/decisions/0011-unicode-range-parsing.md](docs/decisions/0011-unicode-range-parsing.md)参照。

### ストリーミング入出力対応

`core::engine::Engine<S: Sink>`が`Mode::Batch`(一括処理)と
`Mode::Streaming`(ストリーミング処理)を選択できる。`Mode::Streaming`では
`<body>`直下のトップレベルブロック要素が確定するたびに、その要素だけを
スタイル計算・レイアウト・ページ分割・PDF書き出し・DOM解放まで処理する
(`feed`が呼ばれるたびに逐次進む)ため、大きなHTMLでもDOM全体をメモリに
載せずに処理できる。処理済み要素の`ComputedStyle`も不要になり次第
解放するため、60,000要素規模のベンチマークでもピークメモリ使用量が
要素数に依らずほぼ一定(約40MB)に保たれる一方、一括処理は要素数にほぼ
比例して増加する(同条件で約516MB)ことを確認済み(詳細は
[docs/decisions/0010-true-streaming-input.md](docs/decisions/0010-true-streaming-input.md)参照)。

`Mode::Streaming`は以下の制約を伴う:

* `<body>`より後に現れる`<style>`/`<link rel=stylesheet>`タグは非サポート
  (エラーになる)
* `<html>`/`<body>`自身に背景色・背景画像・枠線がある場合は非サポート
* `nth-last-child`等、後続要素への参照が必要なセレクタは常に非マッチになる
* フォントは`--font`または`@font-face`で明示する必要がある
  (システムフォントの自動探索は`Mode::Batch`のみ)

出力側は`Sink` trait(`write`/`finish`)で抽象化されており、Rack
レスポンスへの同期書き込み・S3マルチパートアップロード向けの
バッファリング(`BufferedSink`)・ファイル/メモリバッファへの書き込み
(テスト用)を同じコアロジックで扱える。

### CSS Fragmentation対応

明示的な改ページ制御として、以下のCSSプロパティに対応している:

* `break-before` / `break-after`: `auto | avoid | always | page`
  (`page`は`always`と同じ効果。旧来の`page-break-before`/`page-break-after`も
  エイリアスとして受理する)
* `break-inside`: `auto | avoid`(`page-break-inside`もエイリアス)
* `orphans` / `widows`: ページ末尾/次ページ先頭に残す最小行数(既定2)

CSSを書かずに使える糖衣属性として`data-page-break="before|after|avoid"`にも
対応している(スタイルシートのルールで個別に上書き可能な、弱い優先度のヒントとして
扱われる)。
