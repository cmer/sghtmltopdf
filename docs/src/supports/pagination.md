# ページ分割

sghtmltopdfはCSS Fragmentationのプロパティでページ分割を制御します。

## 明示的な改ページ

```css
.chapter { break-before: page; }   /* この要素の前で改ページ */
.summary { break-after: page; }    /* この要素の後で改ページ */
.card    { break-inside: avoid; }  /* この要素をページ境界で割らない */
```

| プロパティ | 受け付ける値 |
|---|---|
| `break-before` / `break-after` | `auto` / `avoid`(`avoid-page`・`avoid-column`も同義) / `always`(`page`も同義) |
| `break-inside` | `auto` / `avoid`(同上) |

古い`page-break-before` / `page-break-after` / `page-break-inside`もエイリアスとして受け付けます。
wkhtmltopdfやwicked_pdf向けに書いた資産をそのまま持ち込めます。

`left`/`right`/`recto`/`verso`(見開き制御)と多段組み関連の値は非対応です。

### CSSを書かずに指定する

HTML属性でも書けます。

```html
<div data-page-break="before">…</div>
<div data-page-break="after">…</div>
<div data-page-break="avoid">…</div>
```

こちらは弱い優先度のヒントとして扱われるので、スタイルシートのルールで個別に上書きできます。

## 段落が泣き別れないようにする

```css
p {
  orphans: 3;   /* ページ末尾に最低3行は残す */
  widows: 3;    /* 次ページの先頭に最低3行は送る */
}
```

1以上の整数で、初期値はどちらも2です。
指定を満たせない場合は段落ごと次のページへ送られます。

## `@page` — 用紙とページ余白

```css
@page {
  size: A4;
  margin: 20mm;

  @top-center    { content: "請求書"; }
  @bottom-center { content: counter(page) " / " counter(pages); }
}

@page :first {
  @bottom-center { content: "表紙"; }
}
```

* `size`はページサイズのキーワード(`A4`/`Letter`等)・`<length>{1,2}`・`landscape`/`portrait`を受け付けます
* CLIのページ設定オプションより`@page`が優先されます(CLI側は初期値)
* margin box(`@top-left-corner`〜`@bottom-right-corner`の16種)には`content`でテキストを置けます。背景色や枠線などの装飾は非対応です
* `counter(page)`は現在のページ番号、`counter(pages)`は総ページ数です

### `@page`の制約

* `size`/`margin`はページごとに変えられません。`:first`/`:left`/`:right`付きの`size`/`margin`宣言はパースされますが適用されず、これらの擬似クラスは margin boxの 内容の出し分けにだけ使えます
* 名前付きページ(`@page intro`と`page: intro`)は非対応です
* margin boxの寸法は簡略化しています。4隅は縦横マージンの交差部分で固定、残り12個は各辺を3等分した均等割りです(`width`指定は無視されます)
* `counter(pages)`は[ストリーミングモード](../usage/cli/streaming.md)では使えません(総ページ数が1パスでは決まらないため)

CLIの`--header-center`などのオプションは、内部的にこの`@page`のmargin boxへマップされます。
両方書いた場合はCSSが勝ちます。

## テーブルのページ分割

1ページに収まらないテーブルは行単位で分割され、複数ページへ流れます。

* `<thead>`の行は2ページ目以降の先頭に繰り返されます。複数行の見出しにも対応し、1ページに収まる表では複製しません
* `<tfoot>`はソース順に関わらずテーブル末尾へ移動しますが、各ページ下端への繰り返しは行いません(最終ページに1回だけ出ます)
* `caption`は`caption-side`に従って、最初(`top`)または最後(`bottom`)の断片に付きます
* 各断片はテーブル自身の背景・枠線を引き継ぎ、`border-collapse: collapse`の枠線統合もページ内でそのまま効きます

既知の限界として、`rowspan`が分割点をまたぐセルは開始行の断片に属し、下部がページからはみ出します(クリップされません)。
行単位の`break-inside: avoid`とorphans/widows相当も未対応です。

## FlexboxとGridの扱い

| レイアウト | ページ分割 |
|---|---|
| `display: flex` | アトミック。途中で分割せず、収まらなければ次ページへ送る |
| `display: grid` | 行単位で分割する(複数行にまたがるアイテムがある境界では分割しない) |
| `display: table` | 行単位で分割する(上記) |

大きなカードを並べる用途では、flexコンテナが丸ごと次ページへ飛ぶことがあります。
分割してほしい場合はGridかテーブルを使ってください。

## よく使う書き方

見出しが単独でページ末尾に残らないようにする:

```css
h2, h3 {
  break-after: avoid;   /* 見出しの直後で改ページしない */
  break-inside: avoid;
}
```

明細の1件が2ページに割れないようにする:

```css
.line-item { break-inside: avoid; }
```

章ごとに必ず改ページする:

```css
section.chapter + section.chapter { break-before: page; }
```

> **Note**
> [ストリーミングモード](../usage/cli/streaming.md)では`:last-child`などの後方参照セレクタが常に非マッチになります。
> 上の例の`+`(隣接兄弟)は使えます。
