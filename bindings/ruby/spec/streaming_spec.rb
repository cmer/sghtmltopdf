# frozen_string_literal: true

# エンジンのストリーミングモード(`streaming: true`)をgemから使う経路。
#
# `chunk_spec.rb`のブロック付き`render`が「組み上がったPDFを確定した
# ページから順に渡す」のに対し、こちらは「HTMLを読みながらページを確定させ、
# そのページぶんのメモリを手放す」モードで、メモリの上限が効いてくる。
RSpec.describe "streaming: true" do
  # 段落主体の文書。高さを固定して、ページ数が環境のフォントに左右されない
  # ようにする。
  def paragraph_html(count: 3_000)
    body = Array.new(count) { |i| "<p>段落 #{i} 本文です。</p>" }.join
    "<html><head><style>p { height: 60px; margin: 0; }</style></head>" \
      "<body>#{body}</body></html>"
  end

  # 非圧縮で出したPDFのページオブジェクトを数える。
  def page_count(pdf)
    pdf.scan(%r{/Type\s*/Page[^s]}).size
  end

  after { Sghtmltopdf.reset_config! }

  it "ブロック無しでもPDFのバイト列を返す" do
    pdf = Sghtmltopdf.render(paragraph_html(count: 100), streaming: true)

    expect(pdf).to start_with("%PDF-")
    expect(pdf).to end_with("%%EOF")
  end

  it "確定したページから順に複数回yieldされる" do
    chunks = []
    Sghtmltopdf.render(paragraph_html, streaming: true, chunk_size: 1024) { |bytes| chunks << bytes }

    expect(chunks.size).to be > 1
    expect(chunks.first).to start_with("%PDF-")
    expect(chunks.last).to end_with("%%EOF")
  end

  it "通常モードと同じページ数になる" do
    html = paragraph_html
    batch = Sghtmltopdf.render(html, no_pdf_compression: true)
    streamed = +"".b
    Sghtmltopdf.render(html, streaming: true, no_pdf_compression: true) { |bytes| streamed << bytes }

    expect(page_count(streamed)).to eq(page_count(batch))
  end

  # 強制改ページはトップレベル要素同士の関係なので、要素を1つずつ処理する
  # ストリーミングモードでは分割器の側で扱う必要がある。
  it "break-afterによる改ページも通常モードと同じになる" do
    body = Array.new(10) { |i| "<div style=\"break-after: page\">ページ #{i}</div>" }.join
    html = "<html><body>#{body}</body></html>"
    batch = Sghtmltopdf.render(html, no_pdf_compression: true)
    streamed = +"".b
    Sghtmltopdf.render(html, streaming: true, no_pdf_compression: true) { |bytes| streamed << bytes }

    expect(page_count(batch)).to eq(10)
    expect(page_count(streamed)).to eq(page_count(batch))
  end

  it "render_to_fileでも使える" do
    require "tmpdir"

    Dir.mktmpdir("sghtmltopdf-streaming") do |dir|
      path = File.join(dir, "out.pdf")
      Sghtmltopdf.render_to_file(paragraph_html(count: 100), path, streaming: true)

      expect(File.binread(path)).to start_with("%PDF-")
    end
  end

  # 文書全体を見ないと決まらないものは、黙って結果を変えずにエラーにする。
  describe "ストリーミングモードの制約" do
    it "--tocはエラーになる" do
      expect { Sghtmltopdf.render(paragraph_html(count: 10), streaming: true, toc: true) }
        .to raise_error(Sghtmltopdf::RenderError, /toc/)
    end

    it "ブロック付きでも同じエラーになる" do
      expect { Sghtmltopdf.render(paragraph_html(count: 10), streaming: true, toc: true) { |_| } }
        .to raise_error(Sghtmltopdf::RenderError, /toc/)
    end
  end

  # ストリーミングモードの目的そのもの。ページを確定するそばから手放せて
  # いなければ、通常モードと同じだけメモリを使ってしまう。
  describe "ピークメモリ" do
    # 変換で増えたぶんのピークRSS(MB)。
    #
    # ピークRSS(VmHWM)は下がらないので、条件ごとに子プロセスを立てる。
    # Ruby自身とHTML文字列のぶんは両モードに等しく乗るので、変換の前後の
    # 差を見て、モードの違いだけが出るようにする。
    def render_growth_mb(options)
      script = <<~RUBY
        require "sghtmltopdf"
        body = Array.new(40_000) { |i| "<p>段落 \#{i} 本文です。</p>" }.join
        html = "<html><head><style>p { height: 60px; margin: 0; }</style></head>" \\
          "<body>\#{body}</body></html>"
        def peak_kib = File.read("/proc/self/status")[/VmHWM:\\s+(\\d+) kB/, 1].to_i
        before = peak_kib
        Sghtmltopdf.render(html, **#{options.inspect}) { |_| }
        puts peak_kib - before
      RUBY
      lib = File.expand_path("../lib", __dir__)
      output = IO.popen([RbConfig.ruby, "-I", lib, "-e", script], &:read)
      Integer(output.strip) / 1024.0
    end

    before do
      skip "VmHWMが読めない環境" unless File.exist?("/proc/self/status")
    end

    it "通常モードより小さくなる" do
      batch = render_growth_mb({})
      streaming = render_growth_mb({streaming: true})

      # 実測は40,000要素で 105MB 対 44MB(0.42倍)。ページを手放せなく
      # なれば通常モードと並ぶので、環境差を見込んだ緩い境目で見る。
      expect(streaming).to be < batch * 0.6
    end
  end
end
