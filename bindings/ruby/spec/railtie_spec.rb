# frozen_string_literal: true

require "open3"

# T341: Railtieの読み込みは`defined?(Rails::Railtie)`でガードしてある。
# 「Railsが無い場合」を同一プロセスでは作れない(他のspecがRailsを読み込む)
# ため、別プロセスで確かめる。
RSpec.describe "Railtieの読み込みガード" do
  ROOT = File.expand_path("..", __dir__)

  def ruby(script)
    out, err, status = Open3.capture3(
      RbConfig.ruby, "-I#{File.join(ROOT, "lib")}", "-rbundler/setup", "-e", script,
      chdir: ROOT
    )
    raise "子プロセスが失敗しました: #{err}" unless status.success?

    out.split("\n")
  end

  it "Railsが無ければRailtieを読み込まない" do
    expect(ruby(<<~RUBY)).to eq(%w[no no])
      require "sghtmltopdf"
      puts defined?(Rails) ? "yes" : "no"
      puts defined?(Sghtmltopdf::Railtie) ? "yes" : "no"
    RUBY
  end

  it "Railsが読み込まれていればRailtieも読み込む" do
    expect(ruby(<<~RUBY)).to eq(%w[yes yes])
      require "rails"
      require "sghtmltopdf"
      puts defined?(Sghtmltopdf::Railtie) ? "yes" : "no"
      puts defined?(Sghtmltopdf::ViewHelpers) ? "yes" : "no"
    RUBY
  end

  it "Railsが無くても変換は動く" do
    expect(ruby(<<~RUBY)).to eq(["%PDF-"])
      require "sghtmltopdf"
      puts Sghtmltopdf.render("<p>hello</p>")[0, 5]
    RUBY
  end
end
