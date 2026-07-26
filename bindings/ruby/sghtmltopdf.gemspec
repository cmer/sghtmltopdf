# frozen_string_literal: true

require_relative "lib/sghtmltopdf/version"

Gem::Specification.new do |spec|
  spec.name = "sghtmltopdf"
  spec.version = Sghtmltopdf::VERSION
  spec.authors = ["yo_waka"]
  spec.email = ["y.wakahara@gmail.com"]

  spec.summary = "Chromium/WebKit/Geckoに依存しないHTML→PDFレンダラー"
  spec.description = <<~DESC
    wkhtmltopdfの後継として、Chromium/WebKit/Geckoに依存しないRust製の
    HTML→PDFレンダリングエンジンをRubyから使うためのバインディング。
    Railsからはwicked_pdf互換のレンダラ(`render pdf: "invoice"`)で使える。
  DESC
  spec.homepage = "https://github.com/waka/sghtmltopdf"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.2.0"

  spec.metadata["source_code_uri"] = spec.homepage
  spec.metadata["rubygems_mfa_required"] = "true"

  spec.files = Dir[
    "lib/**/*.rb",
    "ext/**/*.{rb,rs,toml}",
    "Cargo.{toml,lock}",
    "LICENSE*",
    "README*"
  ]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/sghtmltopdf/extconf.rb"]
end
