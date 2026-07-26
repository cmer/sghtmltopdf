# frozen_string_literal: true

module Sghtmltopdf
  # グローバルな既定オプション。
  #
  #   Sghtmltopdf.configure do |c|
  #     c.page_size   = "A4"
  #     c.gothic_font = "/path/to/NotoSansJP-Regular.ttf"
  #   end
  #
  # ここで設定した値は`render`/`render_to_file`の引数で上書きできる
  # (マージ順はグローバル → 呼び出し時。
  # docs/decisions/0062-ruby-binding.md 決定7)。
  #
  # **キー名の妥当性は検査しない**。オプション定義はRust側(`cli/options.rs`)の
  # 1箇所に集約する方針(決定2)のため、未知のキーはレンダリング時にclapが
  # `UsageError`として報告する。
  class Configuration
    def initialize(options = {})
      @options = {}
      options.each { |key, value| self[key] = value }
    end

    def [](key)
      @options[key.to_sym]
    end

    def []=(key, value)
      @options[key.to_sym] = value
    end

    def to_h
      @options.dup
    end

    # まだ設定されていないキーにだけ値を入れる。Railtieが
    # Rails向けの既定値を流し込むのに使う(T344)。
    def apply_defaults(defaults)
      defaults.each { |key, value| self[key] = value unless @options.key?(key.to_sym) }
      self
    end

    # `c.page_size = "A4"`と`c.page_size`を受ける。
    def method_missing(name, *args)
      key = name.to_s
      if key.end_with?("=")
        raise ArgumentError, "#{name}は引数1つを取ります" unless args.size == 1

        self[key.chomp("=")] = args.first
      else
        raise ArgumentError, "#{name}は引数を取りません" unless args.empty?

        self[key]
      end
    end

    def respond_to_missing?(_name, _include_private = false)
      true
    end
  end
end
