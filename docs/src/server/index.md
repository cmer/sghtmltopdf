# HTTPサーバモード

常駐させて、HTTP経由で変換を受け付けるモードです。アプリケーションから
プロセスを起動する必要がなくなり、負荷分散もロードバランサに任せられます。

```sh
sghtmltopdf server --listen 127.0.0.1:8080 --font NotoSansJP-Regular.ttf
# → 標準出力に `listening on 127.0.0.1:8080` が出る
```

```sh
curl --data-binary @invoice.html \
     'http://127.0.0.1:8080/pdf?page-size=A4&margin-top=20mm&toc' \
     -o invoice.pdf
```

## 起動オプション

| オプション | 既定 | 説明 |
|---|---|---|
| `--listen <ADDR:PORT>` | `127.0.0.1:8080` | 待ち受けアドレス。`:0`で空きポートを自動割り当て |
| `--workers <N>` | CPUコア数 | 同時に変換するワーカースレッド数 |
| `--max-queue <N>` | ワーカー数×4 | 受理待ちの上限。超えると503 |
| `--max-body-size <BYTES>` | 10485760 (10MiB) | リクエストボディの上限 |
| `--timeout <SECS>` | 30 | **キュー待ちの**上限秒数(超えると504) |
| `--font <PATH>` ほかフォント指定 | | リクエストからは変更できない |
| `--enable-local-file-access` / `--allow <PATH>` / `--allow-remote-assets` | すべて禁止 | 明示的に許可する場合のみ |

> **認証とTLSは持ちません。** 外部へ公開する場合はリバースプロキシを前段に
> 置いてください。

## エンドポイント

| メソッド・パス | 説明 |
|---|---|
| `POST /pdf` | ボディのHTMLをPDFへ変換して返す(`application/pdf`) |
| `POST /pdf?stream=1` | 同上。**chunked transfer encoding**でページが確定したそばから流す |
| `GET /healthz` | `ok` |
| `GET /version` | `sghtmltopdf <version>` |

## クエリパラメータ

**CLIのロングオプションから`--`を取った名前**がそのまま使えます。値の解釈も
CLIと同一です(同じパーサへ通しているため)。

```
?page-size=A4                 →  --page-size A4
?margin-top=20mm              →  --margin-top 20mm
?toc                          →  --toc          (値なしは真)
?grayscale=1  /  =true        →  --grayscale
?grayscale=0  /  =false       →  (指定なしと同じ)
```

値は**パーセントエンコード**できます(`%XX`と`+`)。

各オプションの意味は[CLIリファレンス](../cli/reference.md)を参照してください。

### リクエストからは指定できないオプション

以下を指定すると`400`を返します。ローカルパスを取るもの・出力先・アクセス制御は
**サーバ起動時にだけ**設定できます。

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
| 504 | キュー待ちが`--timeout`超過 |

## ストリーミング

* **入力**: リクエストボディは読み切らずに、64KiBずつエンジンへ流します。
  大きなHTMLを丸ごとメモリに載せません
* **出力**: 既定はバッファ返却(`Content-Length`付き)。`?stream=1`を付けると
  chunked transfer encodingで、ページが確定したそばから流します

```sh
curl --data-binary @big.html 'http://127.0.0.1:8080/pdf?stream=1' -o out.pdf
```

エンジン側の[ストリーミングモード](../cli/streaming.md)(`?streaming`)と併用
すると、入力・レンダリング・出力のすべてが逐次処理になります。

## 既知の限界

* `--timeout`は**キュー待ち時間にしか効きません**。変換処理は同期で
  キャンセル点を持たないため、走り始めた変換は打ち切れません
* `?stream=1`のとき、**ヘッダ送信後に失敗してもステータスは200のまま**に
  なります(パイプが閉じ、クライアントには不完全なPDFが届きます)。クエリの
  不正・空ボディ・サイズ超過はヘッダ送信前に検出するので400/413で返ります
* `?stream=1`のときは**入力を先に読み切ります**(HTTPライブラリの制約で、
  ボディ読み取りと応答が排他のため)。入力と出力のストリーミングは同時には
  使えません

## Railsから使う

Ruby gemには、変換をこのサーバへ委譲する`server_url`設定があります。
[Ruby / Rails](../ruby/index.md#サーバへ委譲する)を参照してください。
