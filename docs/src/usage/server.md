# HTTPサーバモード

常駐させて、HTTP経由で変換を受け付けるモードです。
アプリケーションからプロセスを起動する必要がなくなり、負荷分散もロードバランサに任せられます。

```sh
sghtmltopdf server --listen 127.0.0.1:8080 --font NotoSansJP-Regular.ttf
# → 標準出力に `listening on 127.0.0.1:8080` が出る
```

```sh
curl --data-binary @invoice.html \
     'http://127.0.0.1:8080/pdf?page-size=A4&margin-top=20mm&toc' \
     -o invoice.pdf
```

常駐させるなら、日本語フォントを同梱した[Dockerイメージ](../getting-started/docker.md)が手軽です(引数なしでこのサーバとして起動します)。

```sh
docker run --rm -p 8080:8080 ghcr.io/waka/sghtmltopdf
```

## 起動オプション

| オプション | 既定 | 説明 |
|---|---|---|
| `--listen <ADDR:PORT>` | `127.0.0.1:8080` | 待ち受けアドレス。`:0`で空きポートを自動割り当て |
| `--workers <N>` | CPUコア数 | 同時に変換するワーカースレッド数 |
| `--max-queue <N>` | ワーカー数×4 | 受理待ちの上限。超えると503 |
| `--max-body-size <BYTES>` | 4194304 (4MiB) | リクエストボディの上限 |
| `--timeout <SECS>` | 30 | 1リクエストに与える秒数。キュー待ちと変換の合計(超えると504) |
| `--font <PATH>` ほかフォント指定 | | リクエストからは変更できない |
| `--enable-local-file-access` / `--allow <PATH>` / `--allow-remote-assets` | すべて禁止 | 明示的に許可する場合のみ |

> 認証とTLSは持ちません。
> 外部へ公開する場合はリバースプロキシを前段に置いてください。

## エンドポイント

| メソッド・パス | 説明 |
|---|---|
| `POST /pdf` | ボディのHTMLをPDFへ変換して返す(`application/pdf`) |
| `POST /pdf?stream=1` | 同上。chunked transfer encodingでページが確定したそばから流す |
| `GET /healthz` | `ok` |
| `GET /version` | `sghtmltopdf <version>` |

## クエリパラメータ

CLIのロングオプションから`--`を取った名前がそのまま使えます。
値の解釈もCLIと同一です(同じパーサへ通しているため)。

指定できるのは許可リストに載っているものだけです。
ページの体裁・PDFのメタデータ・ヘッダー/フッターの文字列・目次の見た目など、リクエストごとに変わってよく、かつサーバのファイルシステムにもネットワークにも触れないオプションが対象です。
それ以外は`400`を返します。

|クエリ|相当するCLIオプション|
|-|-|
|`?page-size=A4`|`--page-size A4`|
|`?margin-top=20mm`|`--margin-top 20mm`|
|`?toc`|`--toc`(値なしは真)|
|`?grayscale=1` / `=true`|`--grayscale`|
|`?grayscale=0` / `=false`|指定なしと同じ|

値はパーセントエンコードできます(`%XX`と`+`)。

各オプションの意味は[CLIリファレンス](./cli/reference.md)を参照してください。

### サーバ起動時にだけ指定できるオプション

以下は許可リストに含まれず、指定すると`400`を返します。
ローカルパスを取るもの・出力先・アクセス制御・ログ設定はサーバ起動時にだけ設定できます。

```
font, font-index, gothic-font, gothic-font-index, serif-font, serif-font-index,
mono-font, mono-font-index, output, cover, header-html, footer-html,
user-style-sheet, base-url, allow, enable-local-file-access,
disable-local-file-access, allow-remote-assets, log-level, quiet
```

## ステータスコード

| コード | 状況 |
|---|---|
| 200 | 成功(`Content-Type: application/pdf`) |
| 400 | 未知/禁止のクエリキー、値の形式不正、ボディが空 |
| 404 | 未知のパス |
| 405 | 使えないメソッド |
| 413 | ボディが`--max-body-size`超過 |
| 500 | レンダリング失敗 |
| 503 | キュー溢れ(`--max-queue`超過) |
| 504 | `--timeout`超過(キュー待ち、または変換が長すぎる) |

## ストリーミング

* 入力: リクエストボディは読み切らずに、64KiBずつエンジンへ流します。
  * 大きなHTMLを丸ごとメモリに載せません
* 出力: 既定はバッファ返却(`Content-Length`付き)。`?stream=1`を付けるとchunked transfer encodingで、ページが確定したそばから流します

```sh
curl --data-binary @big.html 'http://127.0.0.1:8080/pdf?stream=1' -o out.pdf
```

エンジン側の[ストリーミングモード](./cli/streaming.md)(`?streaming`)と併用すると、入力・レンダリング・出力のすべてが逐次処理になります。

## メモリの見積もり

変換に必要なメモリは、入力の大きさにほぼ比例します。
実測(最適化ビルド)では次のとおりでした。

| 要因 | 単価 | 抑えているもの |
|---|---|---|
| 要素の数 | 472B〜1210B/ノード | ノード数上限(50万) |
| テキストの量 | 約185MiB/入力1MiB | `--max-body-size` |

どちらの上限も、既定では最悪およそ600〜750MiBに収まる値にしてあります。
ワーカーは同時に変換するので、プロセス全体では「ワーカー数 × この値」が必要です。
既定(`--workers`はCPUコア数)のまま8コアの機械で動かすと最悪6GiB程度になるため、コンテナのメモリ制限に合わせて`--workers`か`--max-body-size`を調整してください。

要素数が上限を超えると`400`を返します。
`?streaming`を付けると処理済みの部分が随時解放されるため、要素数の上限には当たりにくくなります。

## 既知の限界

* `--timeout`はキュー待ちと変換の合計に効きます。変換の打ち切り判定はチャンク投入ごと・トップレベル要素ごと・ページ書き出しごとに行うため、超過に気づくのは最大でその1区間ぶん遅れます。レイアウトの1回の呼び出しの内側までは見ません
* 実測では、10MiBの重いHTMLに`--timeout 2`を指定した場合の実際の応答は2.1〜5.2秒でした(打ち切り後に大きなDOMを破棄する時間も含みます)。指定より早く返ることはありません
* `?stream=1`のとき、ヘッダ送信後に失敗してもステータスは200のままになります(パイプが閉じ、クライアントには不完全なPDFが届きます)。クエリの不正・空ボディ・サイズ超過はヘッダ送信前に検出するので400/413で返ります
* `?stream=1`のときは入力を先に読み切ります(HTTPライブラリの制約で、ボディ読み取りと応答が排他のため)。入力と出力のストリーミングは同時には使えません

## Railsから使う

Ruby gemには、変換をこのサーバへ委譲する`server_url`設定があります。
[Ruby / Rails](./ruby_rails.md#サーバへ委譲する)を参照してください。
