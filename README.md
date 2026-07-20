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

## 開発状況

マイルストーン1(静的HTML一括変換)・マイルストーン2(CSS Fragmentation)・
マイルストーン3(ストリーミング入出力対応)・マイルストーン4(Webfont対応)・
マイルストーン5(画像埋め込み)・マイルストーン6(外部スタイルシート)・
マイルストーン7(`@import`/`background-image`のurl()対応)
ともに完了。マイルストーン8(CSS2.1対応)は、Positioning & Float
(`float`/`clear`/`position: relative`)・Typography詳細・Table layout完全対応
まで完了(Listsは未着手)。

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

### `background-image`

`url(...)`によるCSSプロパティ値としての背景画像指定に対応している
(`<img>`の埋め込みと同じフェッチ層・SSRF対策・`--allow-remote-assets`
フラグを共有する)。

* `background-position`/`background-size`/`background-repeat`/
  `background-attachment`は非対応(非目標)。指定された画像はborder-box
  全体を覆うよう常にストレッチ表示するのみの最小実装
* `border-radius`が指定されていても、背景画像は角丸にクリップされず
  常に直線の矩形として描画される(既知の簡略化)
* 取得・デコードに失敗した背景画像は、その要素だけ背景画像なし扱いにし、
  文書全体の生成は止めない(`<img>`と同じフォールバック方針)
* 背景色→背景画像→枠線の順で描画する

詳細は[docs/decisions/0017-background-image-design.md](docs/decisions/0017-background-image-design.md)
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
