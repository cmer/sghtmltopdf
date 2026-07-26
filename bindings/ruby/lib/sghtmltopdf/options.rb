# frozen_string_literal: true

module Sghtmltopdf
  # オプションハッシュをCLIの引数列(argv)へ変換する。
  #
  # 変換したargvは、CLI・HTTPサーバとまったく同じclapパーサへ渡される
  # (docs/decisions/0062-ruby-binding.md 決定2)。**オプション名の妥当性検査は
  # ここでは行わない**。ホワイトリストを持つとオプションを足すたびに2箇所を
  # 直すことになるため、未知のオプションはclap側のエラーに委ねる。
  module Options
    # 入力は常に標準入力を表す`-`を置く(実際のバイト列はFFIで直接渡すため
    # 読まれない)。出力先はRust側のSinkが決めるので、ここもダミーの`-`。
    # `-`入力のときCLIは`--output`を必須にするため、省略はできない。
    ARGV_PREFIX = ["sghtmltopdf", "-", "--output", "-"].freeze

    module_function

    # @param options [Hash] Rubyのオプションハッシュ
    # @return [Array<String>] clapへ渡す引数列
    def to_argv(options)
      argv = ARGV_PREFIX.dup
      options.each { |key, value| argv.concat(args_for(key, value)) }
      argv
    end

    # 1つのキーと値をargvの断片へ変換する。
    #
    #   page_size: "A4"     → ["--page-size", "A4"]
    #   grayscale: true     → ["--grayscale"]
    #   grayscale: false    → []
    #   allow: ["/a", "/b"] → ["--allow", "/a", "--allow", "/b"]
    def args_for(key, value)
      name = flag_name(key)
      return font_args(value) if name == "font"

      case value
      when nil, false then []
      when true then ["--#{name}"]
      # 配列は同じオプションの繰り返し。要素ごとに同じ規則を適用する。
      when Array then value.flat_map { |element| args_for(key, element) }
      when Hash
        # wicked_pdfの`margin: {top: 10}`のような入れ子は受けない。
        # 数値の単位の解釈が違う(wicked_pdfはmm・こちらはpx)ため、
        # 機械的に平坦化すると黙って別の余白になる。移行時は
        # docs/wicked_pdf_migration.mdの対応表を見て書き換えてもらう。
        example = value.keys.first
        raise ArgumentError,
          "#{key}にHashは渡せません(pathとindexを取るのは:fontだけです)。" \
          "入れ子のオプションは平坦なキーで指定してください" \
          "#{": 例 #{key}_#{example}: \"…\"" if example}"
      else ["--#{name}", value.to_s]
      end
    end

    # `--font`と`--font-index`は**出現順で対応付けられる**
    # (docs/decisions/0055-cli-design.md 決定7。CLIは`ArgMatches#indices_of`で
    # 「`--font-index`より手前にある最後の`--font`」へ結び付ける)。
    # そのため、フェイス番号は必ず対応する`--font`の直後へ置く。
    #
    #   font: "a.ttf"                        → ["--font", "a.ttf"]
    #   font: {path: "a.ttc", index: 1}      → ["--font", "a.ttc", "--font-index", "1"]
    #   font: ["a.ttf", {path: "b.ttc", index: 2}]
    #     → ["--font", "a.ttf", "--font", "b.ttc", "--font-index", "2"]
    def font_args(value)
      case value
      when nil, false then []
      when Array then value.flat_map { |element| font_args(element) }
      when Hash
        path = value[:path] || value["path"]
        raise ArgumentError, "fontのHashにはpathが必要です: #{value.inspect}" if path.nil?

        index = value[:index] || value["index"]
        args = ["--font", path.to_s]
        args.push("--font-index", index.to_s) unless index.nil?
        args
      else ["--font", value.to_s]
      end
    end

    # `:page_size` → `page-size`。
    def flag_name(key)
      key.to_s.tr("_", "-")
    end
  end
end
