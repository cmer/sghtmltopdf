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
マイルストーン3(ストリーミング入出力対応)ともに完了。

### ストリーミング入出力対応

`core::engine::Engine<S: Sink>`が`Mode::Batch`(一括処理)と
`Mode::Streaming`(ストリーミング処理)を選択できる。`Mode::Streaming`では
`<body>`直下のトップレベルブロック要素が確定するたびに、その要素だけを
スタイル計算・レイアウト・ページ分割・PDF書き出し・DOM解放まで処理する
(`feed`が呼ばれるたびに逐次進む)ため、大きなHTMLでもDOM全体をメモリに
載せずに処理できる。60,000要素規模のベンチマークで、一括処理に対し
ピークメモリ使用量を約3.9倍削減することを確認済み(詳細は
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
