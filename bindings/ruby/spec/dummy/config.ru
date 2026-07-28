# frozen_string_literal: true

# 手動確認用にdummyアプリをWebサーバで起動する。
#
#   bundle exec rackup spec/dummy/config.ru
#   → http://localhost:9292/invoices/show
#
# specの`spec/rails_helper.rb`と同じアプリだが、あちらはRSpec前提で
# ブート直後に`reset_config!`する(specの独立性のため)。ここでは
# Railtieが入れた既定値をそのまま残したいので、独立して組んでいる。
ENV["RAILS_ENV"] ||= "development"

require "bundler/setup"
require "logger"
require "rails"
require "action_controller/railtie"
require "sghtmltopdf"
require "sghtmltopdf/railtie"

module DummyServer
  class Application < ::Rails::Application
    config.load_defaults("#{::Rails::VERSION::MAJOR}.#{::Rails::VERSION::MINOR}")
    config.root = __dir__
    config.eager_load = false
    config.secret_key_base = "sghtmltopdf" * 8
    config.logger = Logger.new($stdout)
    # 例外はブラウザにバックトレースを出す。
    config.consider_all_requests_local = true
    config.hosts.clear
  end
end

Rails.application.initialize!

run Rails.application
