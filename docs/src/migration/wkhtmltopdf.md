# wkhtmltopdfからの移行

オプション名の多くはwkhtmltopdfと同じです。
ただし同じ名前でも結果が違うものがいくつかあるので、確認してください。

全オプションの対応状況は[wkhtmltopdfオプション対応表](wkhtmltopdf-options.md)にあります。
Railsでwicked_pdf経由で使っている場合は[wicked_pdfからの移行](wicked-pdf.md)を参照してください。

## 挙動が違うところ

| | wkhtmltopdf | sghtmltopdf |
|---|---|---|
| CLIオプションとCSSの`@page` | CLIが勝つ | `@page`が勝つ(CLIは初期値) |
| マージンの既定値 | 四辺10mm | 四辺1in(96px) |
| 表紙・目次の指定 | 位置引数(`cover a.html toc`) | `--cover <PATH>` / `--toc` |
| 複数HTMLの結合 | できる | できない(入力は1つ) |
| ヘッダー/フッターのページ変数 | JavaScriptで差し込み | プレースホルダ置換(JSは実行しない) |
| フォント | システムフォント | 同じ(`--font`で明示もできる) |
| 非対応オプション | 黙って無視されることがある | 理由を示してexit 1で止まる |

### `@page`が勝つ

もっとも引っかかりやすい違いです。
CSSに`@page { margin: 0 }`と書いてあると、`--margin-top 20mm`は無視されます。

```css
@page { size: A4; margin: 20mm; }   /* こちらが勝つ */
```

CLIオプションは「CSSに書かれていなかったときの初期値」として働きます。
CLIで制御したい場合は、HTMLから`@page`の該当プロパティを外してください。

### マージンの既定値

wkhtmltopdfは四辺10mm、sghtmltopdfは四辺1インチ(96px = 25.4mm)です。
wkhtmltopdfの`--extended-help`は左右にしか既定値を書いていませんが、実際には上下にも10mmが入ります。
指定なしで変換すると余白が変わるので、既存の見た目を保つには明示します。

```sh
sghtmltopdf in.html -o out.pdf \
  --margin-top 10mm --margin-bottom 10mm --margin-left 10mm --margin-right 10mm
```

### 表紙と目次

```sh
wkhtmltopdf cover cover.html toc page.html out.pdf          # wkhtmltopdf
sghtmltopdf --cover cover.html --toc page.html -o out.pdf   # sghtmltopdf
```

目次の見た目はwkhtmltopdfの既定TOC XSLの出力に合わせてあります。
XSLTは非対応なので、変更は`--user-style-sheet`のCSSで行います。

### ヘッダー/フッターのページ番号

wkhtmltopdfは`--header-html`のURLにクエリ(`?page=1&topage=5`)を付け、ページ側のJavaScriptで差し込む方式でした。
JSを実行しないので、sghtmltopdfはプレースホルダの文字列置換に置き換えています。

```sh
sghtmltopdf report.html --footer-center "[page] / [topage]"
```

HTMLでヘッダーを作る場合も、HTMLのテキストとして`[page]`が置換されます。

## 非対応のオプションは黙って無視しない

指定すると理由と代替手段を示して`exit 1`で終了します。
移行時に「オプションが効いていないことに気づかない」事故を避けるためです。

主なものは以下です。

* JavaScript関連: `--enable-javascript`・`--javascript-delay`・`--run-script`・`--window-status`・`--debug-javascript`・`--stop-slow-scripts`(JS実行は設計上の非目標)
* PDFアウトライン: `--outline`・`--outline-depth`・`--dump-outline`
* XSLT: `--xsl-style-sheet`・`--dump-default-toc-xsl`(目次は内蔵テンプレート + CSSで代替)
* 画像の再エンコード: `--image-quality`・`--image-dpi`
* ネットワーク: `--proxy`・`--cookie`・`--custom-header`・`--username`/`--password`・`--ssl-*`
* WebKit固有: `--disable-smart-shrinking`・`--viewport-size`・`--lowquality`・`--print-media-type`(常に印刷メディア扱い)
* PDFフォーム: `--enable-forms`

## HTML/CSS側で必要になる調整

エンジンが別物なので、CSSの対応範囲も違います。
移行時によく当たるのは次の3つです。

1. `!important`が使えません。付いた宣言は無視されます
2. `inherit`/`initial`/`unset`が使えません
3. ビューポート単位(`vw`/`vh`等)と`ex`/`ch`/`lh`が使えません。長さは`px`/`em`/`rem`と絶対単位(`mm`/`cm`/`in`/`pt`/`pc`/`Q`)で書いてください

詳しくは[セレクタ・値・at-rule](../supports/selectors.md)を参照してください。

## 移行できたか確かめる

まずは`--log-level info`(既定)のまま変換し、警告が出ないことを確認します。
非対応オプションはexit 1で止まるので、変換が通った時点でオプションはすべて解釈されています。
あとは出力を目視で比べて、余白・改ページ位置・フォントを確認してください。
