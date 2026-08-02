# frozen_string_literal: true

# CLI・コア・gemはすべて同じバージョンで出す(T351)。リリース時に片方を上げ
# 忘れる事故を、普段のspecで拾う。
RSpec.describe "バージョン" do
  ROOT = File.expand_path("../../..", __dir__)

  def cargo_version(relative_path)
    path = File.join(ROOT, relative_path)
    skip "#{relative_path}が無い(gemとして配布された状態では見えない)" unless File.exist?(path)

    # `[package]`セクションの最初のversion行。
    File.read(path)[/^\s*version\s*=\s*"([^"]+)"/, 1]
  end

  it "Rustコアとgemのバージョンが揃っている" do
    expect(cargo_version("core/Cargo.toml")).to eq(Sghtmltopdf::VERSION)
  end

  it "ネイティブ拡張のクレートとgemのバージョンが揃っている" do
    expect(cargo_version("bindings/ruby/ext/sghtmltopdf/Cargo.toml")).to eq(Sghtmltopdf::VERSION)
  end

  it "CHANGELOGがある" do
    skip "リポジトリ外" unless File.exist?(File.join(ROOT, "core"))

    expect(File.exist?(File.join(ROOT, "CHANGELOG.md"))).to be(true)
  end
end
