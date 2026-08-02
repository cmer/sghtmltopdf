# frozen_string_literal: true

require "rails/railtie"
require_relative "renderer"
require_relative "view_helpers"

module Sghtmltopdf
  # Rails統合。
  #
  # `sghtmltopdf.rb`が`defined?(Rails::Railtie)`のときだけこのファイルを
  # 読み込むため、素のRuby/Sinatraからの利用には影響しない。
  class Railtie < ::Rails::Railtie
    # Railsアプリ向けの既定オプション(T344)。
    #
    # * `base_url`: `Rails.root/public`。PDFのレンダリングはHTTPサーバを
    #   介さないので、`/assets/…`のような絶対パス参照はここを基準に
    #   ローカルファイルとして解決される(precompile済みの本番環境なら
    #   素の`stylesheet_link_tag`がそのまま動く)。開発環境向けには
    #   [ViewHelpers#sghtmltopdf_stylesheet_link_tag]を用意している
    # * `allow`: `Rails.root`。ローカル参照をアプリのディレクトリ配下へ
    #   限定する(CLIのローカルファイルアクセス制御)。テンプレートに
    #   ユーザー入力が混ざっても`/etc/passwd`のような文書外のファイルを
    #   読ませない。
    #   `--font`系で渡すフォントはフェッチャを通らないのでこの制限を受けない
    #
    # どちらも`Sghtmltopdf.configure`で上書きできる(このイニシャライザは
    # `config/initializers/*`より前に走るため、後から設定した値が勝つ)。
    def self.default_options(root)
      root = root.to_s
      defaults = {allow: [root]}
      public_dir = File.join(root, "public")
      defaults[:base_url] = public_dir if File.directory?(public_dir)
      defaults
    end

    initializer "sghtmltopdf.defaults" do |app|
      Sghtmltopdf.config.apply_defaults(Sghtmltopdf::Railtie.default_options(app.root))
    end

    initializer "sghtmltopdf.renderer" do
      ActiveSupport.on_load(:action_controller) do
        Sghtmltopdf::Renderer.register!
      end

      ActiveSupport.on_load(:action_view) do
        include Sghtmltopdf::ViewHelpers
      end
    end
  end
end
