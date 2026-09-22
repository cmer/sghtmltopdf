# frozen_string_literal: true

require "open3"

# The path taken in an app with an asset pipeline (Propshaft) installed.
#
# The dummy app itself runs without a pipeline gem (to check the defaults of a
# plain Rails app), so an app that loads the pipeline is booted in a separate
# process. Same approach as `spec/railtie_spec.rb`.
#
# What we want to see here is the development situation: nothing has been
# precompiled, so `public/assets` is empty and the real files live in
# `app/assets`.
RSpec.describe "an app with an asset pipeline" do
  PIPELINE_ROOT = File.expand_path("..", __dir__)

  # Boots the dummy app with Propshaft in a child process and evaluates `script`.
  def in_pipeline_app(script)
    boot = <<~RUBY
      ENV["RAILS_ENV"] = "test"
      require "logger"
      require "rails"
      require "action_controller/railtie"
      require "propshaft"
      require "sghtmltopdf"
      require "sghtmltopdf/railtie"

      module PipelineDummy
        class Application < ::Rails::Application
          config.load_defaults("\#{::Rails::VERSION::MAJOR}.\#{::Rails::VERSION::MINOR}")
          config.root = #{File.join(PIPELINE_ROOT, "spec/dummy").inspect}
          config.eager_load = false
          config.secret_key_base = "sghtmltopdf" * 8
          config.logger = Logger.new(IO::NULL)
          config.hosts.clear
        end
      end
      Rails.application.initialize!

      view = ActionController::Base.helpers
      root = Rails.root.to_s
    RUBY

    out, err, status = Open3.capture3(
      RbConfig.ruby, "-I#{File.join(PIPELINE_ROOT, "lib")}", "-rbundler/setup",
      "-e", boot + script, chdir: PIPELINE_ROOT
    )
    raise "the child process failed: #{err}" unless status.success?

    out.split("\n")
  end

  it "includes the pipeline load paths in the default allow_path" do
    lines = in_pipeline_app(<<~RUBY)
      allow = Sghtmltopdf.config[:allow_path]
      puts allow.include?(File.join(root, "public"))
      puts allow.include?(File.join(root, "app/assets/images"))
      # Asset paths provided by gems are included too (outside Rails.root).
      puts allow.any? { |dir| !dir.start_with?(root) }
      # config/ is no longer within the readable range.
      puts allow.none? { |dir| dir == root }
    RUBY

    expect(lines).to eq(%w[true true true true])
  end

  # Propshaft raises MissingAssetError when given an asset that is not on its
  # load path. A file that exists only under `public/` is exactly that, so
  # passing it straight through makes `from_public_dir` fail with an exception.
  it "resolves files that exist only under public/ through the pipeline" do
    lines = in_pipeline_app(<<~RUBY)
      puts view.sghtmltopdf_asset_path("logo.png") == File.join(root, "public/logo.png")
      puts view.sghtmltopdf_asset_path("pipeline-logo.png") ==
        File.join(root, "app/assets/images/pipeline-logo.png")
      puts view.sghtmltopdf_asset_path("no-such-file.png").nil?
    RUBY

    expect(lines).to eq(%w[true true true])
  end

  # Plain `image_tag` only emits a digested virtual path, and in dev there is
  # no matching real file anywhere. The helper looks up the load path and
  # points at the actual file.
  it "has no real file behind the virtual path emitted by plain image_tag" do
    lines = in_pipeline_app(<<~RUBY)
      src = view.image_tag("pipeline-logo.png")[/src="([^"]+)"/, 1]
      puts src.start_with?("/assets/pipeline-logo-")
      puts File.file?(File.join(root, "public", src))
      puts File.file?(src)
    RUBY

    expect(lines).to eq(%w[true false false])
  end

  it "points at images outside public/ by absolute path so the engine can read them" do
    lines = in_pipeline_app(<<~RUBY)
      html = view.sghtmltopdf_image_tag("pipeline-logo.png")
      src = html[/src="([^"]+)"/, 1]
      puts src == File.join(root, "app/assets/images/pipeline-logo.png")
      puts html.include?("data:")
      # The 20x16 PNG is embedded as an XObject.
      puts Sghtmltopdf.render(html).include?("/Width 20")
    RUBY

    expect(lines).to eq(%w[true false true])
  end

  # In development the helper reads the uncompiled CSS, so `url()` has not
  # been rewritten by the pipeline and remains a logical path. The engine can
  # only resolve against the document's base_url, so we look up the load path
  # here and repoint it at the actual file.
  it "points url() in pipeline CSS at the actual file through the load path" do
    lines = in_pipeline_app(<<~'RUBY')
      html = view.sghtmltopdf_stylesheet_link_tag("pipeline")
      src = html[/url\("([^"]+)"\)/, 1]
      puts src == File.join(root, "app/assets/images/pipeline-logo.png")
      puts html.include?("data:")
      # The 20x16 PNG is embedded as an XObject.
      puts Sghtmltopdf.render(html + "<p>x</p>").include?("/Width 20")
    RUBY

    expect(lines).to eq(%w[true false true])
  end

  # In dev `public/assets` is empty, so `/assets/…` is read as a logical path
  # on the load path. The mount point (`config.assets.prefix`) is not part of
  # the logical path.
  it "strips the mount point from /assets/ references and looks them up on the load path" do
    lines = in_pipeline_app(<<~'RUBY')
      require "fileutils"
      css = File.join(root, "public/rooted.css")
      begin
        File.write(css, %(body { background-image: url("/assets/pipeline-logo.png"); }))
        html = view.sghtmltopdf_stylesheet_link_tag("rooted")
        puts html.include?(File.join(root, "app/assets/images/pipeline-logo.png"))
      ensure
        FileUtils.rm_f(css)
      end
    RUBY

    expect(lines).to eq(%w[true])
  end

  it "falls back to embedding for url() in CSS when allow_path is narrowed" do
    lines = in_pipeline_app(<<~RUBY)
      Sghtmltopdf.configure { |c| c.allow_path = [File.join(root, "public")] }
      html = view.sghtmltopdf_stylesheet_link_tag("pipeline")
      puts html.include?("url(\\"data:image/png;base64,")
    RUBY

    expect(lines).to eq(%w[true])
  end

  it "falls back to embedding when a narrowed allow_path makes the file unreadable" do
    lines = in_pipeline_app(<<~RUBY)
      Sghtmltopdf.configure { |c| c.allow_path = [File.join(root, "public")] }
      html = view.sghtmltopdf_image_tag("pipeline-logo.png")
      puts html.include?("data:image/png;base64,")
      # Files under public/ stay as relative paths.
      puts view.sghtmltopdf_image_tag("logo.png") == %(<img src="logo.png">)
    RUBY

    expect(lines).to eq(%w[true true])
  end
end
