# frozen_string_literal: true

require "rails_helper"
require "tmpdir"

# ダミーのRailsアプリ(spec/dummy)のコントローラからPDFが返ること。
RSpec.describe "Railsのコントローラ", type: :rails do
  describe "render pdf:" do
    it "PDFを返す" do
      get "/invoices/show"

      expect(last_response.status).to eq(200)
      expect(last_response.headers["content-type"]).to start_with("application/pdf")
      expect(last_response.body).to start_with("%PDF-")
      expect(last_response.body).to end_with("%%EOF")
    end

    it "既定のContent-Dispositionはinlineで、pdf:の値がファイル名になる" do
      get "/invoices/show"

      expect(last_response.headers["content-disposition"])
        .to start_with('inline; filename="invoice.pdf"')
    end

    it "ビューの描画結果がそのまま変換される" do
      get "/invoices/show"
      html = InvoicesController.render(template: "invoices/show", layout: false)

      expect(normalize(last_response.body)).to eq(normalize(Sghtmltopdf.render(html)))
    end
  end

  describe "オプションの受け渡し" do
    it "filename/dispositionがレスポンスに出る" do
      get "/invoices/download"

      disposition = last_response.headers["content-disposition"]
      expect(disposition).to start_with("attachment;")
      # 日本語のファイル名はRFC 5987のfilename*としても出る。
      expect(disposition).to include("filename*=UTF-8''")
    end

    it "変換オプションはPDFへ渡る" do
      get "/invoices/download"
      a5 = last_response.body
      get "/invoices/show"
      a4 = last_response.body

      # download側は page_size: "A5"。用紙サイズが違えば中身も違う。
      expect(normalize(a5)).not_to eq(normalize(a4))
    end

    it "layout:が効く" do
      get "/invoices/with_layout"
      with_layout = last_response.body
      get "/invoices/show"
      without_layout = last_response.body

      expect(normalize(with_layout)).not_to eq(normalize(without_layout))
    end

    it "show_as_htmlならHTMLを返す" do
      get "/invoices/as_html"

      expect(last_response.headers["content-type"]).to start_with("text/html")
      expect(last_response.body).to include("<h1>Invoice #1234</h1>")
    end

    it "未知のオプションはSghtmltopdf::UsageErrorになる" do
      expect { get "/invoices/bad_option" }
        .to raise_error(Sghtmltopdf::UsageError, /--no-such-option/)
    end
  end

  describe "Rails向けの既定オプション" do
    it "Railtieがbase_urlとallowを入れる" do
      expect(CONFIG_AFTER_BOOT[:base_url]).to eq(Rails.root.join("public").to_s)
      expect(CONFIG_AFTER_BOOT[:allow]).to eq([Rails.root.to_s])
    end

    it "config/initializersなど後からの設定で上書きできる" do
      Sghtmltopdf.configure { |c| c.base_url = "/somewhere/else" }

      expect(Sghtmltopdf.config[:base_url]).to eq("/somewhere/else")
    end

    it "base_urlの既定でpublic/のCSSが解決される" do
      html = '<link rel="stylesheet" href="/invoice.css"><h1>Invoice</h1>'
      # 既定(Rails.root/public)ならinvoice.cssが読める。空のディレクトリを
      # base_urlにすると読めない(取得失敗は既定で無視される)。
      resolved = Sghtmltopdf.render(html)
      missing = Dir.mktmpdir { |dir| Sghtmltopdf.render(html, base_url: dir) }

      expect(normalize(resolved)).not_to eq(normalize(missing))
    end

    it "allowの既定ではRails.rootの外のファイルを読まない" do
      Dir.mktmpdir do |dir|
        File.write(File.join(dir, "outside.css"), "h1 { font-size: 48px }")
        html = '<link rel="stylesheet" href="outside.css"><h1>Invoice</h1>'

        blocked = Sghtmltopdf.render(html, base_url: dir)
        allowed = Sghtmltopdf.render(html, base_url: dir, allow: [dir])

        expect(normalize(blocked)).not_to eq(normalize(allowed))
      end
    end
  end

  # ブロック付きrenderをActionController::Liveと組み合わせて、
  # 確定したページから順にRackのレスポンスへ流せること。
  #
  # Rack::Testの`last_response.body`は使えない。`MockResponse`は
  # ストリーミングのボディを読み切らずに最初のチャンクで止まるため、
  # Rackのボディを自分で`each`する。
  describe "Rackへのストリーミング" do
    def stream_response(path)
      status, headers, body = app.call(Rack::MockRequest.env_for(path))
      chunks = []
      body.each { |part| chunks << part }
      body.close if body.respond_to?(:close)
      [status, headers, chunks]
    end

    it "response.streamへチャンクごとに書き出される" do
      status, headers, chunks = stream_response("/streams/show")

      expect(status).to eq(200)
      expect(headers["content-type"]).to start_with("application/pdf")
      # 一括で1回書き出しているのではないこと。
      expect(chunks.size).to be > 1
      expect(chunks.first).to start_with("%PDF-")
      expect(chunks.last).to end_with("%%EOF")
    end

    it "一括変換と同じPDFになる" do
      _status, _headers, chunks = stream_response("/streams/show")
      html = StreamsController.render(template: "invoices/long", layout: false)

      expect(normalize(chunks.join)).to eq(normalize(Sghtmltopdf.render(html)))
    end
  end

  describe "サーバモードへの委譲" do
    it "コントローラからでもサーバへ委譲でき、Railsの既定値は送らない" do
      FakeServer.run do |server|
        Sghtmltopdf.configure { |c| c.server_url = server.url }
        get "/invoices/show"

        expect(last_response.status).to eq(200)
        expect(last_response.body).to start_with("%PDF-")
        # Railtieが入れる`base_url`/`allow`はサーバでは指定できないキーなので、
        # 送ってしまうと400になる。
        expect(server.last_request.query).to eq("")
        expect(server.last_request.body).to include("<h1>Invoice #1234</h1>")
      end
    end
  end

  describe "ビューヘルパ" do
    it "public/のCSSを<style>へ展開する" do
      get "/invoices/with_stylesheet"
      inlined = last_response.body

      # ヘルパを通したPDFは、CSSが当たっていない同じHTMLとは異なる。
      plain = Sghtmltopdf.render("<h1>Invoice</h1>")

      expect(normalize(inlined)).not_to eq(normalize(plain))
    end

    it "見つからないアセットはnilを返す" do
      view = InvoicesController.new.view_context

      expect(view.sghtmltopdf_asset_path("no-such-file.css")).to be_nil
      expect(view.sghtmltopdf_asset_path("invoice.css")).to eq(Rails.root.join("public/invoice.css").to_s)
    end
  end
end
