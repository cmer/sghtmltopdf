# frozen_string_literal: true

module Sghtmltopdf
  # `render pdf: "invoice"`のオプションを、
  #
  # * Railsのビュー描画(`render_to_string`)へ渡すもの
  # * レスポンスの組み立て(`send_data`)へ渡すもの
  # * PDF変換(`Sghtmltopdf.render`)へ渡すもの
  #
  # の3つに振り分ける。
  #
  # 振り分けは「Railsのキーとレスポンスのキーだけを列挙し、残りは全部変換
  # オプションとみなす」方式にしている。変換オプションのホワイトリストを
  # 持たないのは、オプション定義をRust側(`cli/options.rs`)の1箇所に集約する
  # 方針のため。未知のキーはclapが`UsageError`として報告する。
  #
  # Railsに依存しないpure Rubyのクラスなので、Rails無しでも単体テストできる。
  class Renderer
    # `render_to_string`へそのまま渡すキー。
    RAILS_RENDER_KEYS = %i[
      action assigns body collection file formats handlers html inline layout
      locals object partial plain prefixes template variants
    ].freeze

    # レスポンスの組み立てに使うキー(`send_data`へ渡す)。
    RESPONSE_KEYS = %i[disposition filename status].freeze

    # レンダラ自身が解釈するキー(PDFにせずHTMLのまま返すデバッグ用)。
    RENDERER_KEYS = %i[show_as_html].freeze

    PDF_CONTENT_TYPE = "application/pdf"
    HTML_CONTENT_TYPE = "text/html"

    attr_reader :name, :options

    # @param name [String, Symbol, nil] `pdf:`に渡された値(ファイル名の素)
    # @param options [Hash] `render`に渡されたその他のオプション
    # @param default_name [String, nil] `name`が空のときのファイル名
    #   (コントローラの`action_name`を想定)
    def initialize(name, options = {}, default_name: nil)
      @name = blank?(name) ? (default_name || "document").to_s : name.to_s
      @options = options.to_h { |key, value| [key.to_sym, value] }
    end

    # `ActionController::Renderers.add(:pdf)`でレンダラを登録する。
    # RailtieのAction Controller読み込みフック(`on_load`)から呼ぶ。
    def self.register!
      ::ActionController::Renderers.add(:pdf) do |name, options|
        renderer = ::Sghtmltopdf::Renderer.new(name, options, default_name: action_name)
        html = render_to_string(**renderer.render_options)
        send_data(renderer.body_for(html), **renderer.send_data_options)
      end
    end

    # ビューの描画に使うオプション。
    def render_options
      options.select { |key, _| RAILS_RENDER_KEYS.include?(key) }
    end

    # PDF変換に使うオプション。グローバル設定とのマージは
    # `Sghtmltopdf.render`側で行う(マージ順はグローバル → ここ)。
    def convert_options
      known = RAILS_RENDER_KEYS + RESPONSE_KEYS + RENDERER_KEYS
      options.reject { |key, _| known.include?(key) }
    end

    # 描画したHTMLをレスポンスの本文へ変換する。
    def body_for(html)
      show_as_html? ? html : Sghtmltopdf.render(html, **convert_options)
    end

    def send_data_options
      opts = {type: content_type, disposition: disposition}
      opts[:filename] = filename unless show_as_html?
      opts[:status] = options[:status] if options.key?(:status)
      opts
    end

    def content_type
      show_as_html? ? HTML_CONTENT_TYPE : PDF_CONTENT_TYPE
    end

    # `filename: "x.pdf"` > `pdf: "x"` の順。拡張子は二重に付けない。
    def filename
      base = blank?(options[:filename]) ? name : options[:filename].to_s
      base.downcase.end_with?(".pdf") ? base : "#{base}.pdf"
    end

    # wicked_pdfと同じく既定は`inline`(ブラウザ内で開く)。
    def disposition
      blank?(options[:disposition]) ? "inline" : options[:disposition].to_s
    end

    # wicked_pdfの`show_as_html`相当。PDFにせずHTMLをそのまま返すので、
    # ブラウザの開発者ツールでレイアウトを確認できる。
    def show_as_html?
      value = options[:show_as_html]
      !(value.nil? || value == false || value == "false")
    end

    private

    def blank?(value)
      value.nil? || (value.respond_to?(:empty?) && value.empty?)
    end
  end
end
