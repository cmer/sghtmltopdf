# フォント

PDFは文書の中にフォントを埋め込みます。ブラウザと違って「見る人の環境に
あるフォントで表示する」ということができないため、**どのフォントを使うかは
変換時に決まります**。

## フォントが決まる順番

1. CLIの`--font`(および`--gothic-font`/`--serif-font`/`--mono-font`)
2. CSSの`@font-face`
3. `font-family`に書かれた名前でのシステムフォント探索

どれでも1つも見つからなかった場合だけ、システムの`sans-serif`候補が既定
フォントになります。

> **サーバやCIでは`--font`を明示してください。** 指定しないと出力が実行環境の
> フォント構成に依存します。同じHTMLが開発機と本番で違う見た目になる、という
> 事故はここから起きます。

## 汎用ファミリー名

`serif` / `sans-serif` / `monospace`はシステムフォントから解決されます
(`cursive` / `fantasy`は解決しません)。

日本語では、この解決を環境任せにすると本文の書体が変わってしまうので、
CLIから決定的に指定できます。

```sh
sghtmltopdf invoice.html \
  --gothic-font NotoSansJP-Regular.ttf \   # font-family: sans-serif の実体
  --serif-font  NotoSerifJP-Regular.ttf \  # font-family: serif の実体
  --mono-font   NotoSansMono-Regular.ttf   # font-family: monospace の実体
```

TrueType Collection(`.ttc`)を使う場合は、直前の`--font`系オプションに対して
`--font-index`でフェイス番号を指定します。

## `@font-face`

```css
@font-face {
  font-family: "MyFont";
  src: url("fonts/MyFont-Regular.ttf");
  font-weight: 400;
  font-style: normal;
}

body { font-family: "MyFont", sans-serif; }
```

対応するディスクリプタは`font-family` / `src` / `unicode-range` /
`font-weight` / `font-style`です。`src`の`local()`と、`format()`/`tech()`付きの
`url()`も受け付けます。`font-display`などその他のディスクリプタは無視されます。

> **フォントファイルはTTF/OTFのみです。WOFF/WOFF2は非対応**なので、Webで
> 配信しているwebfontをそのまま指すとエラーになります。元のTTF/OTFを使って
> ください。

**読み込みの待ち合わせはありません。** headless Chromeで必要だった
`document.fonts.ready`待ちのような処理は不要で、フォントが未解決のまま
PDF化されることはありません。

## `unicode-range`

文字の範囲ごとにフォントを切り替えられます。英数字は欧文フォント、日本語は
和文フォント、という典型的な構成がそのまま書けます。

```css
@font-face {
  font-family: "Mixed";
  src: url("fonts/Latin.ttf");
  unicode-range: U+0-24F, U+1E00-1EFF;
}
@font-face {
  font-family: "Mixed";
  src: url("fonts/JP.ttf");            /* 上の範囲外はこちら */
}
```

* 単一コードポイント・範囲・ワイルドカード(`U+4??`)・カンマ区切りの複数指定に
  対応します
* 宣言された範囲は**ハードフィルタ**として働きます。範囲外の文字には、その
  フォントが実際にグリフを持っていても使いません
* `unicode-range`を書かなかったフォント(`local()`・`--font`・システム探索を
  含む)は全域をカバーします
* 範囲が重なった場合は、CSSの中で**先に宣言されたほう**が優先されます

## 太字と斜体

| 指定 | 挙動 |
|---|---|
| `font-weight` | `normal`/`bold`/`100`〜`900`。数値は600以上を`bold`とみなす2値化。太字のフォントが無い場合は、塗りに縁取りを足した**疑似ボールド**で描画します |
| `font-style` | `normal`/`italic`/`oblique`(`oblique`は`italic`と同一視)。イタリック字形が無い場合は、テキスト行列のせん断による**疑似イタリック**になります |

`font`ショートハンドは非対応です。`font-size`・`font-family`などの
ロングハンドを個別に書いてください。

## サブセット化

埋め込まれるのは**実際に使ったグリフだけ**です。日本語フォントを丸ごと
指定しても、PDFのサイズは文書に出てくる文字の分にしかなりません。

## ストリーミングモードでの注意

[ストリーミングモード](../cli/streaming.md)では、`font-family`名からの
**システムフォント自動探索が行われません**(警告を出して既定フォントで描画
します)。`--font`系オプションか`@font-face`で明示すれば、ストリーミングでも
意図どおりのフォントになります。
