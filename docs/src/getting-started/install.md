# インストール

使い方に応じて3通りあります。

| 使い方 | 入れるもの |
|---|---|
| コマンドラインで変換する / [HTTPサーバ](../server/index.md)を立てる | 実行ファイル`sghtmltopdf` |
| Ruby・Railsから使う | gem `sghtmltopdf` |

> **Note**
> 最初のリリース(v0.1.0)前のため、**rubygems.orgへのgem公開とDockerイメージの
> 配布はまだ行われていません**。現時点ではソースからビルドしてください。

## ソースからビルド

必要なのは[Rustのstableツールチェイン](https://rustup.rs/)だけです。C言語の
ライブラリやシステムパッケージへの依存はありません。

```sh
git clone https://github.com/waka/sghtmltopdf.git
cd sghtmltopdf
cargo build --release
```

実行ファイルは`target/release/sghtmltopdf`にできます。パスの通った場所へ
置くか、そのまま呼び出してください。

```sh
./target/release/sghtmltopdf --version
```

HTTPサーバモードが要らない場合は、featureを削って小さくできます。

```sh
cargo build --release --no-default-features --features cli
```

## Ruby / Rails

```ruby
# Gemfile
gem "sghtmltopdf"
```

ビルド済み(precompiled)のgemを配布する方針のため、**利用側にRustの
ツールチェインは要りません**。対応は`x86_64-linux`・`aarch64-linux`・
`arm64-darwin`(glibc)と、Ruby 3.2以上です。

外部プロセスは起動せず、ネイティブ拡張(magnus + rb-sys)として**同じプロセスの
中で**変換します。重い処理の間はGVLを解放するので、Pumaの他のスレッドは
止まりません。

使い方は[Ruby / Rails](../ruby/index.md)を参照してください。

## フォントについて

`--font`を指定しない場合、システムにインストールされたフォントが使われます。
手元で試すぶんには問題ありませんが、**サーバやCIでは出力が実行環境の
フォント構成に依存します**。日本語を含む文書では、フォントファイルを明示するか、
コンテナに同梱することを推奨します。

```sh
sghtmltopdf invoice.html \
  --font NotoSansJP-Regular.ttf \
  --gothic-font NotoSansJP-Regular.ttf
```

詳しくは[フォント](../css/fonts.md)を参照してください。
