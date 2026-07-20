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
マイルストーン5(画像埋め込み)ともに完了。

### 画像埋め込み

`<img>`要素(JPEG/PNG/WebP)の埋め込みに対応している。ローカル相対パス
(`base_dir`基準)・`http(s)`絶対URL・`data:` URIのいずれの`src`にも対応する。

* JPEGはデコードせず、SOF0/SOF2マーカーからwidth/height/コンポーネント数
  だけを読んでDCTDecodeフィルタでそのまま埋め込む(再エンコードなし)。
  PNG/WebPはフルデコードし、アルファチャンネルがあれば`/SMask`付きの
  透過画像として埋め込む
* リモート画像フェッチは既定無効(CLIの`--allow-remote-images`で
  オプトイン)。有効化してもプライベート/loopback/link-local
  (クラウドメタデータの`169.254.169.254`含む)IP宛のリクエストは常に
  ブロックする(SSRF対策。DNSリバインディング・リダイレクト経由の
  バイパスも同じ仕組みで防ぐ)
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

* `<body>`より後に現れる`<style>`タグは非サポート(エラーになる)
* `<html>`/`<body>`自身に背景色・枠線がある場合は非サポート
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
