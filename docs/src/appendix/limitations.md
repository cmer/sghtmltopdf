# 対応していないこと

個別のCSSプロパティの可否は[プロパティ対応表](../supports/properties.md)を、wkhtmltopdfのオプション単位の可否は[オプション対応表](../migration/wkhtmltopdf-options.md)を参照してください。

## JavaScriptを実行しない

`<script>`は読み飛ばされます。

そのため次のような使い方はできません。

* JSでDOMを組み立ててからPDF化する(SPAのページをそのまま出す等)
* Chart.jsなどでクライアント描画したグラフを含める
* JSでページ番号やヘッダーを差し込む(→ [プレースホルダ](../usage/cli/reference.md#ヘッダーフッター)を使ってください)

グラフを載せたい場合は、サーバ側で画像(PNG)を生成して`<img>`で埋め込むか、CSSで描ける範囲の表現に置き換えてください。

## 入力は1つのHTML

複数のHTMLファイルを並べて1つのPDFへ結合することはできません(wkhtmltopdfの位置引数に相当する機能がありません)。
表紙は`--cover`、目次は`--toc`で個別に指定します。

既存のPDF同士の結合・分割・ページ抽出も対象外です。

## PDFの機能

| 機能 | 状況 |
|---|---|
| アウトライン(しおり・ブックマーク) | 非対応。文書内の目次は`--toc`で作れます |
| 入力可能なフォーム(AcroForm) | 非対応。`<input>`等は見た目だけ描画します |
| 暗号化・パスワード・電子署名 | 非対応 |
| PDF/A・PDF/X などの規格準拠 | 非対応 |
| タグ付きPDF(アクセシビリティ) | 非対応 |
| 添付ファイル・注釈(リンク以外) | 非対応。リンク注釈のみ対応 |

リンク(`<a href>`)は、外部URL・文書内の`#id`ともPDFの注釈になります。

## 画像・フォントの形式

* 画像はPNG / JPEG / WebPのみ。SVGとGIFは非対応です
* フォントはTTF / OTFのみ。WOFF / WOFF2は非対応です
* `--grayscale`を指定しても、JPEGとCMYK画像はカラーのまま残ります

## CSSの主な制限

機能単位では以下が非対応です。

* 縦書き(`writing-mode`/`text-orientation`)と論理プロパティ(`margin-inline`等)、`direction`による右横書き
* 多段組み(`columns`/`column-count`)
* グラデーション(`linear-gradient()`等)と複数背景
* アニメーション・トランジション・`filter`(静的な出力のため)
* `position: sticky`、`display: inline-flex`/`inline-grid`、subgrid
* `:is()`/`:where()`/`:has()`、`::first-line`、`::marker`

## ストリーミングモード固有の制限

`--streaming`を使う場合は、総ページ数(`counter(pages)`・`[topage]`)や目次(`--toc`)などが使えなくなります。
詳細は[ストリーミングモード](../usage/cli/streaming.md)を参照してください。

## 今後について

JavaScriptエンジンの組み込みは、必要性が出てきた段階での検討事項として残してあります。
上記のうちPDFのアウトラインなど、設計上の非目標ではないものは将来対応する可能性があります。
