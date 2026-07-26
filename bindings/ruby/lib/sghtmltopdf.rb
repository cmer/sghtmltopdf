# frozen_string_literal: true

require_relative "sghtmltopdf/version"
require_relative "sghtmltopdf/options"
require_relative "sghtmltopdf/configuration"
require_relative "sghtmltopdf/renderer"

# precompiled gemはRubyのマイナーバージョンごとのディレクトリへ`.so`を置く
# (rake-compilerのクロスビルドの慣習)。開発中の`rake compile`は
# `lib/sghtmltopdf/sghtmltopdf.so`に置くので、両方を試す。
begin
  RUBY_VERSION =~ /(\d+\.\d+)/
  require "sghtmltopdf/#{Regexp.last_match(1)}/sghtmltopdf"
rescue LoadError
  require "sghtmltopdf/sghtmltopdf"
end

# Chromium/WebKit/Geckoに依存しないHTML→PDFレンダラー。
#
#   pdf = Sghtmltopdf.render("<h1>請求書</h1>", page_size: "A4")
#   Sghtmltopdf.render_to_file(html, "invoice.pdf", margin_top: "20mm")
#
# オプション名はCLI(`sghtmltopdf --help`)と同じで、`_`が`-`に対応する
# (`page_size:` → `--page-size`)。詳細はdocs/cli.mdを参照。
#
# 例外は`Sghtmltopdf::Error`を基底に、`UsageError`(オプションの誤り)・
# `InputError`(入力やファイルの読み書き)・`RenderError`(レンダリング失敗)の
# 3つ(ネイティブ拡張側で定義。docs/decisions/0062-ruby-binding.md 決定9)。
module Sghtmltopdf
  class << self
    # HTMLを変換してPDFのバイト列(ASCII-8BITのString)を返す。
    def render(html, **options)
      Native.render(html.to_s, argv_for(options))
    end

    # HTMLを変換して`path`へ書き出す。
    #
    # 一時ファイルへ書いて成功時だけrenameするため、途中で失敗しても
    # 壊れたPDFが出力先に残らない。
    def render_to_file(html, path, **options)
      Native.render_to_file(html.to_s, argv_for(options), path.to_s)
      nil
    end

    # グローバルな既定オプション。
    def configure
      yield config
      config
    end

    def config
      @config ||= Configuration.new
    end

    # 主にテスト用。設定を空に戻す。
    def reset_config!
      @config = Configuration.new
    end

    private

    # グローバル設定 → 呼び出し時オプションの順にマージしてargvにする。
    def argv_for(options)
      Options.to_argv(config.to_h.merge(options))
    end
  end
end

# Rails統合(Railtie・`render pdf:`・ビューヘルパ)は、Railsが読み込まれて
# いるときだけ有効にする(docs/decisions/0062-ruby-binding.md 決定1)。
# 通常のRailsアプリでは`config/application.rb`の`rails/all`が先に走るため、
# Bundler.requireでこのファイルが読まれた時点で定数が揃っている。
require_relative "sghtmltopdf/railtie" if defined?(::Rails::Railtie)
