# frozen_string_literal: true

module Sghtmltopdf
  # Action View用のヘルパ。
  #
  # PDFのレンダリングはHTTPサーバを介さないため、`/assets/…`のようなURLは
  # ローカルファイルとして解決される(`--base-url`の既定は
  # `Rails.root/public`。[Railtie.default_options])。precompile済みの
  # 本番環境ではこれで素の`stylesheet_link_tag`もそのまま動くが、開発環境の
  # ようにアセットがまだ`public/`へ書き出されていない場合は解決できない。
  #
  # そこで、
  #
  #   <%= sghtmltopdf_stylesheet_link_tag "pdf" %>
  #
  # のようにCSSの中身を`<style>`へ展開するヘルパを用意する(wicked_pdfの
  # `wicked_pdf_stylesheet_link_tag`に相当)。
  module ViewHelpers
    # アセットのローカルファイルパスを返す。見つからなければ`nil`。
    #
    # 1. `public/`配下(precompile済み。本番環境)
    # 2. アセットパイプラインのロードパス(開発環境。Propshaft/Sprockets)
    #
    # の順に探す。パイプラインの参照はどちらのgemにも依存しないよう
    # `respond_to?`で分岐している(best effort)。
    def sghtmltopdf_asset_path(source)
      path = source.to_s
      return path if path.start_with?("/") && File.file?(path)

      from_public_dir(path) || from_asset_pipeline(path)
    end

    # CSSの中身を`<style>`へ展開する。複数指定でき、見つからないものは
    # 黙って飛ばす(PDF生成そのものは止めない)。
    def sghtmltopdf_stylesheet_link_tag(*sources)
      css = sources.flatten.filter_map do |source|
        path = sghtmltopdf_asset_path(with_extension(source, ".css"))
        File.read(path) if path
      end
      return "".html_safe if css.empty?

      content_tag(:style, css.join("\n").html_safe, type: "text/css")
    end

    # `image_tag`のsrcをローカルファイルパスへ差し替える。
    def sghtmltopdf_image_tag(source, options = {})
      image_tag(sghtmltopdf_asset_path(source) || source, options)
    end

    private

    # `asset_path`が返すURL(asset_hostが付くこともある)からパス部分だけを
    # 取り出し、`public/`配下の実ファイルへ対応付ける。
    def from_public_dir(source)
      url = respond_to?(:asset_path) ? asset_path(source) : source
      relative = url.to_s.sub(%r{\Ahttps?://[^/]+}, "").split(/[?#]/).first.to_s
      return nil if relative.empty?

      candidate = File.join(::Rails.public_path.to_s, relative)
      File.file?(candidate) ? candidate : nil
    end

    def from_asset_pipeline(source)
      assets = ::Rails.application.try(:assets)
      return nil if assets.nil?

      # Propshaft
      if assets.respond_to?(:load_path)
        found = assets.load_path.find(source)
        return found.path.to_s if found.respond_to?(:path)
      end
      # Sprockets
      if assets.respond_to?(:[])
        found = assets[source]
        return found.filename.to_s if found.respond_to?(:filename)
      end
      nil
    end

    def with_extension(source, extension)
      name = source.to_s
      name.end_with?(extension) ? name : "#{name}#{extension}"
    end
  end
end
