# frozen_string_literal: true

require "rails/railtie"
require_relative "renderer"
require_relative "view_helpers"

module Sghtmltopdf
  class Railtie < ::Rails::Railtie
    # The directories the engine may read from: `public/`, where the
    # precompiled assets are, plus the asset pipeline load paths, where their
    # sources are in development. The pipeline paths reach outside `Rails.root`
    # for the assets a gem or an engine provides, which is why the whole of
    # `Rails.root` is not a substitute for them.
    #
    # `config.assets` raises rather than answering `nil` when no pipeline gem is
    # installed, hence `try`.
    def self.default_options(app)
      public_dir = File.join(app.root.to_s, "public")
      pipeline = Array(app.config.try(:assets)&.paths).map(&:to_s)
      allow = [public_dir, *pipeline].select { |dir| File.directory?(dir) }.uniq

      defaults = {}
      # An empty list would mean "no --allow-path", which is not the same thing as
      # "allow nothing": it hands the boundary back to `base_url`. Leaving the
      # key out says that, and says it in one place.
      defaults[:allow_path] = allow unless allow.empty?
      defaults[:base_url] = public_dir if File.directory?(public_dir)
      defaults
    end

    # Read in `after_initialize`, not inside the initializer. The pipeline fills
    # `config.assets.paths` in its own initializer (for Propshaft,
    # `propshaft.append_assets_path`), and that one runs later.
    #
    # This runs after `config/initializers`, but `apply_defaults` is always
    # weaker than explicitly set values, so it never overrides user settings.
    initializer "sghtmltopdf.defaults" do |app|
      app.config.after_initialize do
        Sghtmltopdf.config.apply_defaults(Sghtmltopdf::Railtie.default_options(app))
      end
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
