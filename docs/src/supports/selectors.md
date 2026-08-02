# セレクタ・値・at-rule対応表

- [セレクタ](#セレクタ)
- [擬似クラス](#擬似クラス)
- [擬似要素](#擬似要素)
- [値・単位・関数](#値単位関数)
- [色](#色)
- [at-rule](#at-rule)
- [ストリーミングモード固有の制約](#ストリーミングモード固有の制約)

## セレクタ

マッチングはServo由来の[`selectors`](https://crates.io/crates/selectors)クレートに委譲しているため、CSS3セレクタは概ねそのまま使える。

| セレクタ | 対応 | 備考 |
| - | - | - |
| タイプ(`p`)・ユニバーサル(`*`) | ✅ | |
| クラス(`.foo`)・ID(`#foo`) | ✅ | |
| 属性(`[a]`/`[a=v]`/`[a^=v]`/`[a$=v]`/`[a*=v]`/`[a~=v]`/`[a\|=v]`) | ✅ | 大文字小文字を無視する`i`フラグも使える |
| 子孫(空白)・子(`>`) | ✅ | |
| 隣接兄弟(`+`)・一般兄弟(`~`) | ✅ | |
| セレクタリスト(`,`) | ✅ | |
| 名前空間(`ns\|E`) | ❌ | `@namespace`自体が非対応 |

## 擬似クラス

| 擬似クラス | 対応 | 備考 |
| - | - | - |
| `:root` | ✅ | |
| `:first-child` / `:last-child` / `:only-child` | ✅ | ストリーミングモードでは`:last-child`が常に非マッチ(下記参照) |
| `:nth-child()` / `:nth-last-child()` | ✅ | 同上(`:nth-last-child()`はストリーミングモードで非マッチ) |
| `:first-of-type` / `:last-of-type` / `:only-of-type` / `:nth-of-type()` / `:nth-last-of-type()` | ✅ | 同上 |
| `:empty` | ✅ | 同上 |
| `:not()` | ✅ | |
| `:is()` / `:where()` / `:has()` | ❌ | パースエラー(セレクタごと無視される) |
| `:hover` / `:active` / `:focus` / `:focus-within` / `:focus-visible` / `:target` / `:enabled` / `:disabled` / `:checked` / `:visited` | ⚠️ | パースは通るが常に非マッチ。対話状態を持たない静的なPDF出力では意味を持たないため |
| `:link` / `:any-link` | ✅ | `href`を持つ`<a>`にマッチする |

非対応の擬似クラスがセレクタに含まれるとルール全体が捨てられる(`:is()`等)。
一方`:hover`のようにパースが通るものは、ルールとしては生き残った上でマッチしない。

## 擬似要素

| 擬似要素 | 対応 | 備考 |
| - | - | - |
| `::before` / `::after` | ⚠️ | `content`による生成テキストのみ。ホスト要素の計算スタイルをそのまま流用して描画し、擬似要素専用のボックススタイル(margin/padding/display等)は持たない。ブロック子を持つ要素では生成されない |
| `::first-letter` | ⚠️ | `font-family`/`font-size`/`font-weight`/`font-style`/`color`/`text-decoration-line`/`text-transform`のみ上書きできる(`float`・box model系は非対応) |
| `::first-line` | ❌ | パースエラー |
| `::marker` / `::selection` / `::placeholder` | ❌ | パースエラー |

## 値・単位・関数

### 長さ

| 単位 | 対応 |
| - | - |
| `px` / `em` / `rem` | ✅ |
| `%`(パーセンテージを取るプロパティで) | ✅ |
| 単位なしの`0` | ✅ |
| `pt` / `pc` / `cm` / `mm` / `in` / `Q` | ❌ |
| `ex` / `ch` / `vw` / `vh` / `vmin` / `vmax` / `lh` | ❌ |

`@page`の`size`だけは例外的にページサイズのキーワード(`a4`/`letter`等)と`landscape`/`portrait`を受け付ける。

### 角度(`transform`専用)

`deg` / `rad` / `grad` / `turn`、および単位なしの`0`に対応 ✅。

### 関数

| 関数 | 対応 | 備考 |
| - | - | - |
| `calc()` | ⚠️ | `+`/`-`/`*`/`/`と括弧のネスト。項に使えるのは`px`/`em`/`rem`/`%`/数値のみ。長さ・パーセンテージを取るプロパティ全般で使える |
| `min()` / `max()` / `clamp()` | ❌ | |
| `var()` | ⚠️ | カスタムプロパティ(`--foo`)をパース前のテキスト置換で解決する。フォールバック(`var(--x, 10px)`)・カスタムプロパティ同士の参照に対応。カスケードや継承には従わず、文書全体で「最後に書かれた宣言が勝つ」単純な解決になる点が本来の仕様と異なる |
| `url()` | ✅ | `background-image`/`list-style-image`/`@font-face`の`src`/`@import`。相対URLは`<base href>`または入力元を基準に解決する |
| `attr()` | ⚠️ | `content`の中でのみ使える |
| `counter()` / `counters()` | ✅ | `content`の中。第2引数のスタイルは`list-style-type`の値集合 |
| `linear-gradient()`等のグラデーション | ❌ | |
| `env()` / `image-set()` / `element()` | ❌ | |

### 色

| 記法 | 対応 |
| - | - |
| 名前付き色(`red`等のCSS色キーワード) | ✅ |
| `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` | ✅ |
| `rgb()` / `rgba()`(カンマ区切り・スペース区切りとも) | ✅ |
| `hsl()` / `hsla()` / `hwb()` | ✅ |
| `lab()` / `lch()` / `oklab()` / `oklch()` | ✅ (sRGBへ変換して描画) |
| `currentcolor` / `transparent` | ✅ |
| `color()`(`color(display-p3 ...)`等) | ❌ |
| `color-mix()` / 相対色構文(`rgb(from ...)`) | ❌ |

アルファ付きの色は塗り・背景ともPDFのExtGStateで透過描画する。

## at-rule

| at-rule | 対応 | 備考 |
| - | - | - |
| `@media` | ⚠️ | メディアタイプのみを評価する。`screen`(および`not screen`以外の否定)はブロックごと無視し、`print`/`all`/タイプ省略は適用する。`(min-width: ...)`のような特性クエリは評価せず読み飛ばす(=タイプさえ合致すれば中身が適用される) |
| `@page` | ⚠️ | `size`(キーワード/`<length>{1,2}`/`landscape`/`portrait`)と`margin`系に対応。`:first`/`:left`/`:right`擬似クラス単体に対応し、名前付きページ(`@page intro`)・`:blank`・複合擬似クラス(`:first:left`)は非対応 |
| `@page`内のmargin box | ⚠️ | `@top-left-corner`/`@top-left`/`@top-center`/`@top-right`…の16種すべて。`content`のテキスト描画のみで、背景色・枠線等の装飾は非対応。`counter(page)`/`counter(pages)`でページ番号を出力できる(`counter(pages)`はストリーミングモードでは非対応) |
| `@font-face` | ⚠️ | `font-family`/`src`/`unicode-range`/`font-weight`/`font-style`ディスクリプタに対応(`src`の`local()`・`format()`/`tech()`付き`url()`も受理)。`font-display`等その他のディスクリプタは無視。フォントファイルはTTF/OTFのみで、WOFF/WOFF2は非対応 |
| `@import` | ✅ | ネスト(深さ上限16、超過分はその1件だけ無視)・循環参照の検出に対応。`@import url(...) screen;`のようなメディアクエリ条件は評価せず常に取り込む |
| `@charset` | ❌ | 入力はUTF-8前提 |
| `@supports` / `@keyframes` / `@namespace` / `@counter-style` / `@layer` / `@container` / `@property` | ❌ | ブロックごと無視される |

## ストリーミングモード固有の制約

`Mode::Batch`(一括処理)ではDOM全体が揃っているため下記の制約は無い。
`Mode::Streaming`でのみ以下が適用される。

* 後方参照セレクタは常に非マッチ: `:last-child`/`:last-of-type`/`:nth-last-child()`/`:nth-last-of-type()`/`:empty`(対象要素の親の子リストが完結するまで原理的に判定できないため)
* `<body>`開始後の`<style>`タグはエラー: `EngineError::UnsupportedInStreamingMode`を返す。
  黙って見た目が崩れるのを避けるため、`<style>`は`<head>`に集約する
* `position: absolute`/`fixed`は無視される
* `counter(pages)`(総ページ数)は使えない
