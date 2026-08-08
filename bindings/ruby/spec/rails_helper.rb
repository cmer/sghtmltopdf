# frozen_string_literal: true

# Rails統合のテスト用。`spec/dummy`を`Rails.root`にした最小の
# アプリを起動する。
ENV["RAILS_ENV"] ||= "test"

require "logger"
require "rails"
require "action_controller/railtie"
require "rack/test"

# 通常のRailsアプリでは`rails/all`のあとにBundler.requireが走るため、
# `sghtmltopdf.rb`のガードでRailtieが読み込まれる。specでは`spec_helper`が
# 先に`sghtmltopdf`を読んでいるので、同じ経路を手で踏む
# (ガードそのものはspec/railtie_spec.rbが別プロセスで確かめる)。
require "sghtmltopdf/railtie"

module Dummy
  class Application < ::Rails::Application
    config.load_defaults("#{::Rails::VERSION::MAJOR}.#{::Rails::VERSION::MINOR}")
    config.root = File.expand_path("dummy", __dir__)
    config.eager_load = false
    config.secret_key_base = "sghtmltopdf" * 8
    config.logger = Logger.new(IO::NULL)
    config.consider_all_requests_local = true
    # 例外はテストへそのまま伝える(500のHTMLに包まない)。
    config.action_dispatch.show_exceptions = :none
    config.hosts.clear
  end
end

Rails.application.initialize!

# Railtieのイニシャライザが流し込んだ既定値。ブート直後の状態を
# 控えてから設定を戻し、Railsを使わないspecへ影響させない
# (specの実行順はランダムで、他のspecは`reset_config!`で設定を空にする)。
CONFIG_AFTER_BOOT = Sghtmltopdf.config.to_h.freeze
Sghtmltopdf.reset_config!

module RailsAppHelpers
  include Rack::Test::Methods

  def app
    Rails.application
  end
end

RSpec.configure do |config|
  config.include RailsAppHelpers, type: :rails
  # 各exampleをブート直後と同じ設定から始める。
  config.before(type: :rails) { Sghtmltopdf.config.apply_defaults(CONFIG_AFTER_BOOT) }
  config.after(type: :rails) { Sghtmltopdf.reset_config! }
end
