# frozen_string_literal: true

# Renderer単体のテスト。Railsを読み込まずに振り分けだけを確かめる。
RSpec.describe Sghtmltopdf::Renderer do
  def renderer(name = "invoice", **options)
    described_class.new(name, options)
  end

  describe "オプションの振り分け" do
    subject(:pdf) { renderer(template: "invoices/show", layout: "pdf", page_size: "A4", margin_top: "20mm") }

    it "Railsのキーはビュー描画へ渡す" do
      expect(pdf.render_options).to eq(template: "invoices/show", layout: "pdf")
    end

    it "残りはすべて変換オプションにする" do
      expect(pdf.convert_options).to eq(page_size: "A4", margin_top: "20mm")
    end

    it "レスポンス用のキーは変換オプションに混ぜない" do
      pdf = renderer(disposition: "attachment", filename: "x.pdf", status: 201, show_as_html: false)
      expect(pdf.convert_options).to be_empty
      expect(pdf.render_options).to be_empty
    end

    it "文字列のキーも受ける" do
      pdf = described_class.new("invoice", {"template" => "a/b", "page_size" => "A4"})
      expect(pdf.render_options).to eq(template: "a/b")
      expect(pdf.convert_options).to eq(page_size: "A4")
    end
  end

  describe "#filename" do
    it "pdf:の値に拡張子を足す" do
      expect(renderer("invoice").filename).to eq("invoice.pdf")
    end

    it "拡張子は二重に付けない" do
      expect(renderer("invoice.pdf").filename).to eq("invoice.pdf")
      expect(renderer("invoice.PDF").filename).to eq("invoice.PDF")
    end

    it "filename:があればそちらが勝つ" do
      expect(renderer("invoice", filename: "請求書").filename).to eq("請求書.pdf")
    end

    it "pdf:が空ならdefault_nameを使う" do
      expect(described_class.new(nil, {}, default_name: "show").filename).to eq("show.pdf")
      expect(described_class.new("", {}, default_name: "show").filename).to eq("show.pdf")
    end

    it "default_nameも無ければdocumentにする" do
      expect(described_class.new(nil, {}).filename).to eq("document.pdf")
    end
  end

  describe "#disposition" do
    it "既定はinline" do
      expect(renderer.disposition).to eq("inline")
    end

    it "指定があればそれを使う" do
      expect(renderer(disposition: :attachment).disposition).to eq("attachment")
    end
  end

  describe "#send_data_options" do
    it "PDFのContent-Typeとファイル名を返す" do
      expect(renderer.send_data_options)
        .to eq(type: "application/pdf", disposition: "inline", filename: "invoice.pdf")
    end

    it "status:はそのまま渡す" do
      expect(renderer(status: 201).send_data_options[:status]).to eq(201)
    end

    it "show_as_htmlならHTMLとして返す(ファイル名は付けない)" do
      expect(renderer(show_as_html: true).send_data_options)
        .to eq(type: "text/html", disposition: "inline")
    end
  end

  describe "#body_for" do
    let(:html) { "<h1>Invoice</h1>" }

    it "既定ではPDFへ変換する" do
      expect(renderer.body_for(html)).to start_with("%PDF-")
    end

    it "変換オプションが効く" do
      a4 = normalize(renderer(page_size: "A4").body_for(html))
      a5 = normalize(renderer(page_size: "A5").body_for(html))
      expect(a4).not_to eq(a5)
    end

    it "show_as_htmlならHTMLをそのまま返す" do
      expect(renderer(show_as_html: true).body_for(html)).to eq(html)
    end
  end
end
