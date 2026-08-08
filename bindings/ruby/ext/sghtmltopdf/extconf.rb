# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

core = File.expand_path("../../../../core", __dir__)
unless File.exist?(File.join(core, "Cargo.toml"))
  abort <<~MESSAGE
    sghtmltopdf: Rustコア(#{core})が見つかりません。

    このgemは対応プラットフォーム向けのprecompiled gemとして配布しています。
    お使いの環境(#{RUBY_PLATFORM} / ruby #{RUBY_VERSION})向けのビルド済みgemが
    無いためソースからのビルドが試みられましたが、ソースgemにはRustコアが
    含まれていないためビルドできません。

    対応プラットフォーム: x86_64-linux / aarch64-linux / x86_64-linux-musl / aarch64-linux-musl / arm64-darwin
  MESSAGE
end

# `lib/sghtmltopdf/sghtmltopdf.so`として作る。
create_rust_makefile("sghtmltopdf/sghtmltopdf")
