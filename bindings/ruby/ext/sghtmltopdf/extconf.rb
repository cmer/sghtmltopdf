# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

# ソースからのビルドはリポジトリのチェックアウト上でしか成立しない。
# gemに詰められるのは`bindings/ruby`配下だけで、extが依存するRustコア
# (`core/`)は親ディレクトリにあるため入らない(gem buildはgemspecのある
# ディレクトリ配下しか集められない)。配布はprecompiled gem
# (docs/decisions/0061-distribution.md 決定2・3)なので通常ここは通らない。
# 分かりにくいコンパイルエラーになる前に理由を出す。
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

# `lib/sghtmltopdf/sghtmltopdf.so`として作る。
# `Init_sghtmltopdf`はCargo.tomlのパッケージ名から生成される
# (docs/decisions/0062-ruby-binding.md「パッケージ名がInit_シンボル名を決める」)。
create_rust_makefile("sghtmltopdf/sghtmltopdf")
