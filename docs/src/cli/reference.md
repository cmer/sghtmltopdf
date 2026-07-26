# CLIリファレンス

`sghtmltopdf`コマンドの全オプション。

```
sghtmltopdf [OPTIONS] <INPUT.HTML>      # 変換
sghtmltopdf server [OPTIONS]            # HTTPサーバ(別ページ)
```

wkhtmltopdfのオプションとの対応(非対応にしたものを含む全一覧)は
[wkhtmltopdfオプション対応表](../migration/wkhtmltopdf-options.md)を参照して
ください。

## 基本

```sh
# もっとも単純な使い方(フォントはシステムのものが使われる)
sghtmltopdf invoice.html -o invoice.pdf

# 出力先を省略すると入力の拡張子を .pdf にしたもの
sghtmltopdf invoice.html

# 標準入力から読み、標準出力へ書く
cat invoice.html | sghtmltopdf - -o - > invoice.pdf
```

## 入出力

| オプション | 既定 | 説明 |
|---|---|---|
| `<INPUT.HTML>` | (必須) | 入力HTML。`-`で標準入力 |
| `-o`, `--output <PATH>` | 入力の拡張子を`.pdf`に | 出力先。`-`で標準出力。標準入力から読む場合は省略できない |
| `--base-url <URL\|DIR>` | 入力HTMLのあるディレクトリ | 相対参照の解決基準。http(s)のURLを渡すと`<base href>`の既定値になる(HTML内の`<base href>`が優先) |
| `--encoding <NAME>` | 自動判定 | 入力の文字エンコーディング。判定順は **BOM > `--encoding` > `<meta charset>` > UTF-8** |
| `--streaming` | オフ | [ストリーミングモード](streaming.md)で処理する |

出力ファイルは一時ファイルへ書いてから`rename`されるため、**失敗したときに
壊れたPDFが残ることはありません**。

## ページ設定

| オプション | 既定 | 説明 |
|---|---|---|
| `-s`, `--page-size <SIZE>` | A4 | `A3`/`A4`/`A5`/`Letter`/`Legal`(大文字小文字を区別しない) |
| `--page-width <LENGTH>` | | 用紙の幅。`--page-size`より優先 |
| `--page-height <LENGTH>` | | 用紙の高さ。`--page-size`より優先 |
| `-O`, `--orientation <O>` | Portrait | `Landscape`は**最後に**幅と高さを入れ替える |
| `-T`, `--margin-top <LENGTH>` | 1in (96px) | 上マージン |
| `-B`, `--margin-bottom <LENGTH>` | 1in | 下マージン |
| `-L`, `--margin-left <LENGTH>` | 1in | 左マージン |
| `-R`, `--margin-right <LENGTH>` | 1in | 右マージン |

長さの単位は`mm`/`cm`/`in`/`pt`/`px`。**単位を省略するとmm**です
(wkhtmltopdf互換)。

> **CSSの`@page`との関係**
> これらのオプションは**初期値**であり、HTMLのCSSに`@page { size: … }`や
> `@page { margin: … }`が書かれていれば**そちらが勝ちます**(プロパティ単位)。
> wkhtmltopdfとは逆なので注意してください。

## フォント

| オプション | 説明 |
|---|---|
| `--font <PATH>` | 使うフォント(複数指定可)。**省略するとシステムフォント**を使う |
| `--font-index <N>` | 直前の`--font`に対する、TrueType Collection(`.ttc`)内のフェイス番号 |
| `--gothic-font <PATH>` (+`--gothic-font-index`) | `font-family: sans-serif`の実体 |
| `--serif-font <PATH>` (+`--serif-font-index`) | `font-family: serif`の実体 |
| `--mono-font <PATH>` (+`--mono-font-index`) | `font-family: monospace`の実体 |

フォントの解決順は「`--font` → `@font-face` → `font-family`名でのシステム探索」。
それでも1つも見つからない場合だけ、システムの`sans-serif`候補が既定フォントに
なります。

> `--font`を指定しないと**出力が実行環境のフォントに依存します**。サーバ運用や
> CIで出力を安定させたい場合は`--font`(または`@font-face`)を明示してください。
> 詳しくは[フォント](../css/fonts.md)を参照。

## PDFの出力形式・メタデータ

| オプション | 既定 | 説明 |
|---|---|---|
| `--title <TEXT>` | HTMLの`<title>` | PDFの`/Title` |
| `--author` / `--subject` / `--keywords <TEXT>` | | Info辞書の各項目(sghtmltopdf独自) |
| `-d`, `--dpi <DPI>` | 96 | CSS pxを何dpiとして解釈するか。`72`にすると1px=1pt |
| `--zoom <FACTOR>` | 1.0 | 拡大率。`--dpi`の係数に掛かる |
| `-g`, `--grayscale` | オフ | 塗り・線をグレースケール化(sRGB相対輝度) |
| `--no-pdf-compression` | オフ | PDFオブジェクトのFlate圧縮を止める(画像データは対象外) |

`/Producer`と`/CreationDate`は常に書かれます。

> **グレースケールの限界**
> JPEG(`/DCTDecode`のパススルー)とCMYK画像はデコーダを持たないため、変換されず
> カラーのまま残ります。

## コンテンツの挙動

| オプション | 説明 |
|---|---|
| `--no-images` | `<img>`とCSS`background-image`を読み込まない |
| `--no-background` | 要素の背景(色・画像)を描かない |
| `--user-style-sheet <PATH>` | ユーザーオリジンのCSS(複数指定可)。UAスタイルより強く、著者CSSより弱い |
| `--minimum-font-size <PX>` | 算出`font-size`の下限 |
| `--disable-external-links` | 外部リンク(http(s))のPDF注釈を作らない |
| `--disable-internal-links` | 内部リンク(`#id`)のPDF注釈を作らない |
| `--keep-relative-links` | 相対URLの外部リンクを絶対化せずそのまま書く |
| `--load-media-error-handling <ignore\|abort>` | 画像・CSS・フォントの取得失敗時の挙動(既定`ignore`) |

## ヘッダー/フッター

2つの方法があります。同じ側に両方を指定した場合は`--header-html`が優先されます。

### 1. テキストで指定する

`@page`のmargin boxへマップされます。

```sh
sghtmltopdf report.html \
  --header-center "四半期レポート" \
  --footer-right "[page] / [topage]" \
  --header-line
```

| オプション | 説明 |
|---|---|
| `--header-left` / `--header-center` / `--header-right <TEXT>` | ヘッダーの3分割位置 |
| `--footer-left` / `--footer-center` / `--footer-right <TEXT>` | フッターの3分割位置 |
| `--header-font-name` / `--header-font-size` | ヘッダーのフォント(footerも同様) |
| `--header-line` / `--footer-line` | 罫線を引く |
| `--header-spacing` / `--footer-spacing <MM>` | 本文との間隔。**その分だけマージンが増える** |
| `--default-header` | タイトルとページ番号の既定ヘッダー |
| `--replace <NAME=VALUE>` | 任意の`[NAME]`を値へ置換(複数指定可) |

**プレースホルダ**は`[page]`(現在ページ)・`[topage]`(総ページ数)・`[frompage]`・
`[title]`/`[doctitle]`・`[date]`・`[time]`、および`--replace`で定義した名前です。

`[section]`/`[subsection]`と`[webpage]`/`[sitepage]`/`[sitepages]`は非対応です。

### 2. HTMLで指定する

```sh
sghtmltopdf report.html --header-html header.html --footer-html footer.html
```

各ページの余白領域へ、別のHTMLをレンダリングして合成します。プレースホルダは
HTMLのテキストとして置換されます(JavaScriptは実行しません)。

* 余白に入りきらない分は**クリップされます**(マージンは自動で広がりません)
* **外部リソースを取得しません**。使えるのはインラインの`<style>`・テキスト・
  枠線・背景色までで、`<img>`と外部CSSは非対応です
* ヘッダー/フッターHTML内の`@font-face`は読み込みません。そこでしか使わない
  フォントは`--font`で明示してください

## 表紙と目次

```sh
sghtmltopdf report.html --cover cover.html --toc --footer-center "[page]"
```

書き出される順は **表紙 → 目次 → 本文** です。

| オプション | 既定 | 説明 |
|---|---|---|
| `--cover <PATH>` | | 表紙にするHTML。**ページ番号に数えず**、ヘッダー/フッターも出さない |
| `--toc` | オフ | 目次を本文の前に挿入する(ストリーミングモードでは使えない) |
| `--toc-header-text <TEXT>` | `Table of Contents` | 目次の`<h1>` |
| `--toc-level-indentation <WIDTH>` | `1em` | 階層1段ごとのインデント |
| `--toc-text-size-shrink <REAL>` | `0.8` | 階層1段ごとの文字サイズ比 |
| `--disable-dotted-lines` | (引く) | 項目の破線の下線を引かない |
| `--disable-toc-links` | (張る) | 目次から見出しへのリンクを張らない |
| `--enable-toc-back-links` | (張らない) | 見出しから目次へ戻るリンクを張る |
| `--page-offset <N>` | 0 | ページ番号の起点をずらす |

目次のHTML構造と既定スタイルは**wkhtmltopdfの既定TOC XSLの出力に合わせて**
あります(階層は入れ子の`<ul>`、各項目は
`<div><a>見出し</a><span>ページ番号</span></div>`)。見た目を変えたい場合は
`--user-style-sheet`でCSSを当ててください(XSLTは非対応)。

見出しは`h1`〜`h6`から集めます。`id`が無い見出しには自動で宛先名が振られます。

## アクセス制御

| オプション | CLIの既定 | サーバの既定 |
|---|---|---|
| `--enable-local-file-access` / `--disable-local-file-access` | 許可 | **禁止** |
| `--allow <PATH>` | 制限なし | 制限なし |
| `--allow-remote-assets` | **禁止** | **禁止** |

`--allow`を1つ以上指定すると、ローカル参照はそのディレクトリ配下だけに限定
されます。`<img src>`・外部CSS・`@font-face`のすべてに効きます。

## ログとexit code

`--log-level <none|error|warn|info>`(既定`info`)、`-q`/`--quiet`
(=`--log-level none`)。

| code | 意味 |
|---|---|
| 0 | 成功 |
| 1 | 使用法エラー(不明なオプション、値の形式不正、非対応オプションの指定) |
| 2 | 入力/リソースエラー(ファイルが無い、フォントが読めない、`abort`指定での取得失敗) |
| 3 | レンダリングエラー(ストリーミングモードの制約違反など) |
