# 画像

`<img>`とCSSの`background-image`で画像を埋め込めます。

| 対応フォーマット | PNG / JPEG / WebP |
|---|---|
| `src`に書けるもの | ローカルの相対パス・絶対パス、`http(s)`のURL、`data:` URI |

SVGとGIFは非対応です。

## `<img>`

```html
<img src="logo.png" width="120">
<img src="https://example.com/chart.png" alt="売上推移">
<img src="data:image/png;base64,iVBORw0…">
```

* `<img>`はインラインの置換要素として行に載ります。独立した行にしたい場合は`display: block`を指定してください
* `width`/`height`属性とCSSの`width`/`height`に対応します。どちらも無指定なら画像の内在サイズを使い、片方だけ指定すればアスペクト比を保って他方を導出します
* 取得やデコードに失敗した画像は、その要素だけ空として扱い、文書全体の生成は止めません(`--load-media-error-handling abort`で中断させることもできます)
* 同じ画像を何度使っても、取得・デコード・PDFへの埋め込みは初回の1回だけです

## `object-fit` / `object-position`

指定した枠に対して画像をどう収めるかを制御します。

```css
img.thumb {
  width: 120px;
  height: 80px;
  object-fit: cover;          /* fill | contain | cover | none | scale-down */
  object-position: 50% 50%;
}
```

## 背景画像

```css
.watermark {
  background-image: url("stamp.png");
  background-position: center;
  background-size: contain;
  background-repeat: no-repeat;
}
```

`background-image`に書けるのは`url()`だけです。
`linear-gradient()`などのグラデーション関数と、カンマ区切りの複数背景は非対応です。
既定では画像の内在サイズでタイル配置されます。

`border-radius`と背景画像を併用した場合、角丸によるクリップは行われません(角丸は背景色の塗りにのみ効きます)。

## リモート画像の取得

既定では無効です。
`--allow-remote-assets`で明示的に有効化します。

```sh
sghtmltopdf report.html --allow-remote-assets
```

有効にした場合も、プライベートIP・ループバック・リンクローカル(クラウドのメタデータエンドポイント`169.254.169.254`を含む)宛のリクエストは
常にブロックされます。
DNSリバインディングやリダイレクト経由の迂回も同じ仕組みで防いでいます。

信頼できないHTMLを変換する場合は、`--allow`でローカル参照の範囲も併せて絞ってください。

```sh
sghtmltopdf untrusted.html --allow /var/app/assets
```

## JPEGはそのまま埋め込まれる

JPEGはデコードせず、サイズ情報だけを読んでPDFへそのまま(DCTDecodeとして)埋め込みます。
再エンコードしないので画質は落ちず、変換も速くなります。

その代わり、`--grayscale`を指定してもJPEGとCMYK画像はカラーのまま残ります(デコーダを持たないため)。
グレースケール化が必要な場合は、変換前の画像をグレースケールにしておいてください。

PNGとWebPはフルデコードし、アルファチャンネルがあれば透過画像として埋め込みます。

## 画像を一切読み込まない

```sh
sghtmltopdf invoice.html --no-images
```

`<img>`とCSSの`background-image`の両方を読み込まなくなります。
