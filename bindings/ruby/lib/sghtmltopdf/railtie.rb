# frozen_string_literal: true

require "rails/railtie"
require_relative "renderer"
require_relative "view_helpers"

module Sghtmltopdf
  class Railtie < ::Rails::Railtie
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
