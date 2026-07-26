# はじめてのPDF

[インストール](install.md)が済んでいる前提で、1枚出すところから改ページまでを
順に試します。

## 1. 変換する

`hello.html`を用意します。

```html
<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8">
    <style>
      body { font-family: sans-serif; }
      h1 { border-bottom: 2px solid #333; padding-bottom: 8px; }
      .total { text-align: right; font-size: 1.2em; font-weight: bold; }
    </style>
  </head>
  <body>
    <h1>請求書</h1>
    <p>いつもお世話になっております。下記のとおりご請求申し上げます。</p>
    <p class="total">合計 ¥12,000</p>
  </body>
</html>
```

変換します。

```sh
sghtmltopdf hello.html -o hello.pdf
```

出力先(`-o`)を省略すると、入力の拡張子を`.pdf`にしたファイル名になります。
`-`を使うと標準入力から読み、標準出力へ書けます。

```sh
cat hello.html | sghtmltopdf - -o - > hello.pdf
```

## 2. 用紙と余白を決める

```sh
sghtmltopdf hello.html -o hello.pdf \
  --page-size A4 --margin-top 20mm --margin-bottom 20mm
```

単位は`mm`/`cm`/`in`/`pt`/`px`が使え、省略すると`mm`です。

同じことはCSSの`@page`でも書けます。**両方書いた場合はCSSが勝ちます**
(CLIオプションは初期値として扱われます)。wkhtmltopdfと逆なので、移行時は
[wkhtmltopdfからの移行](../migration/wkhtmltopdf.md)を確認してください。

```css
@page {
  size: A4;
  margin: 20mm;
}
```

## 3. 改ページする

「ここから次のページ」は、CSSの`break-before`で指定します。

```html
<style>
  .page-break { break-before: page; }
</style>

<h1>請求書</h1>
<p>1ページ目です。</p>

<div class="page-break">
  <h1>明細</h1>
  <p>2ページ目です。</p>
</div>
```

`break-after`(この要素の後で改ページ)、`break-inside: avoid`(この要素を
分割しない)も使えます。見出しだけが行末に取り残されるのを防ぐ`orphans`/
`widows`もあります。詳しくは[ページ分割](../css/pagination.md)を参照してください。

## 4. ヘッダーとフッターを付ける

```sh
sghtmltopdf hello.html -o hello.pdf \
  --header-center "請求書" \
  --footer-right "[page] / [topage]" \
  --header-line
```

`[page]`は現在のページ番号、`[topage]`は総ページ数に置き換わります。
JavaScriptは実行しないため、この置換で表現します。HTMLでヘッダーを
作りたい場合は`--header-html`が使えます。

## 5. フォントを固定する

ここまでの例はシステムのフォントを使っています。サーバやCIで**出力を環境に
依存させたくない場合は、フォントファイルを明示します**。

```sh
sghtmltopdf hello.html -o hello.pdf \
  --font NotoSansJP-Regular.ttf \
  --gothic-font NotoSansJP-Regular.ttf
```

## 次に読むもの

* オプションの全体像は[CLIリファレンス](../cli/reference.md)
* サーバとして常駐させるなら[サーバモード](../server/index.md)
* Railsから使うなら[Ruby / Rails](../ruby/index.md)
* どこまでCSSが効くかは[先に読んでほしい規則](../css/rules.md)
