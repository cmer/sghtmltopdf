# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

# ソースからのビルドはリポジトリのチェックアウト上でしか成立しない。gemに
# 詰められるのは`bindings/ruby`配下だけで、extが依存するRustコア(`core/`)は親
# ディレクトリにあるため入らない(gem buildはgemspecのあるディレクトリ配下しか
# 集められない)。配布はprecompiled gemなので通常ここは通らない。分かりにくい
# コンパイルエラーになる前に理由を出す。
core = File.expand_path("../../../../core", __dir__)
unless File.exist?(File.join(core, "Cargo.toml"))
  abort <<~MESSAGE
    sghtmltopdf: Rustコア(#{core})が見つかりません。

    このgemは対応プラットフォーム向けのprecompiled gemとして配布しています。
    お使いの環境(#{RUBY_PLATFORM} / ruby #{RUBY_VERSION})向けのビルド済みgemが
    無いためソースからのビルドが試みられましたが、ソースgemにはRustコアが
    含まれていないためビルドできません。

    対応プラットフォーム: x86_64-linux / aarch64-linux / arm64-darwin
    ソースから使いたい場合はリポジトリを取得してください:
      https://github.com/waka/sghtmltopdf
  MESSAGE
end

# `lib/sghtmltopdf/sghtmltopdf.so`として作る。`Init_sghtmltopdf`はCargo.tomlの
# パッケージ名から生成される。
create_rust_makefile("sghtmltopdf/sghtmltopdf")
