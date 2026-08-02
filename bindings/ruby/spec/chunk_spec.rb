# frozen_string_literal: true

# ローカル変換(ネイティブ拡張)でのチャンク出力(M14 Phase 6)。
RSpec.describe "ブロック付きrender(ローカル変換)" do
  # 複数ページになるHTML。ページ確定ごとにSinkへ書き出されるので、
  # チャンクが複数回に分かれる。
  def multipage_html(pages: 12)
    body = Array.new(pages) do |i|
      "<div style=\"break-after: page\"><h1>Page #{i + 1}</h1>" \
        "<p>#{"本文です。" * 40}</p></div>"
    end.join
    "<html><head><title>chunks</title></head><body>#{body}</body></html>"
  end

  after { Sghtmltopdf.reset_config! }

  it "確定したページから順に複数回yieldされる" do
    chunks = []
    Sghtmltopdf.render(multipage_html, chunk_size: 1024) { |bytes| chunks << bytes }

    expect(chunks.size).to be > 1
    expect(chunks.first).to start_with("%PDF-")
    expect(chunks.last).to end_with("%%EOF")
  end

  it "結合すると一括renderと同じPDFになる" do
    html = multipage_html
    chunks = []
    Sghtmltopdf.render(html, chunk_size: 1024) { |bytes| chunks << bytes }

    expect(normalize(chunks.join)).to eq(normalize(Sghtmltopdf.render(html)))
  end

  it "返り値はnil" do
    expect(Sghtmltopdf.render("<p>x</p>") { |_| }).to be_nil
  end

  it "チャンクはASCII-8BITで渡ってくる" do
    encodings = []
    Sghtmltopdf.render(multipage_html, chunk_size: 1024) { |bytes| encodings << bytes.encoding }

    expect(encodings.uniq).to eq([Encoding::ASCII_8BIT])
  end

  describe "chunk_size" do
    it "小さくするとチャンク数が増える" do
      html = multipage_html
      few = 0
      many = 0
      Sghtmltopdf.render(html, chunk_size: 64 * 1024) { |_| few += 1 }
      Sghtmltopdf.render(html, chunk_size: 512) { |_| many += 1 }

      expect(many).to be > few
    end

    it "既定は64KiB" do
      expect(Sghtmltopdf::DEFAULT_CHUNK_SIZE).to eq(64 * 1024)
    end

    it "グローバル設定でも指定できる" do
      Sghtmltopdf.configure { |c| c.chunk_size = 512 }
      chunks = 0
      Sghtmltopdf.render(multipage_html) { |_| chunks += 1 }

      expect(chunks).to be > 1
    end

    it "変換オプションとしては渡さない(clapは知らないキー)" do
      expect { Sghtmltopdf.render("<p>x</p>", chunk_size: 512) { |_| } }.not_to raise_error
    end
  end

  describe "ブロックが中断したとき" do
    it "投げた例外がそのまま伝播する" do
      expect { Sghtmltopdf.render(multipage_html, chunk_size: 512) { |_| raise ArgumentError, "止める" } }
        .to raise_error(ArgumentError, "止める")
    end

    it "例外のあとGCが走ってもVMが壊れない" do
      10.times do
        Sghtmltopdf.render(multipage_html, chunk_size: 512) { |_| raise "止める" }
      rescue RuntimeError
        nil
      end
      GC.start

      # 壊れていればここまでに落ちている。続けて変換できることも確かめる。
      expect(Sghtmltopdf.render("<p>ok</p>")).to start_with("%PDF-")
    end

    it "GC.stressの下でも例外が正しく伝播する" do
      # チャンクごとにRubyのStringを作るので、ここでGCが動く。
      # ブロックやスタック上の値の扱いを誤っていれば落ちる。
      GC.stress = true
      begin
        expect { Sghtmltopdf.render("<p>x</p>", chunk_size: 512) { |_| raise IndexError, "止める" } }
          .to raise_error(IndexError, "止める")
      ensure
        GC.stress = false
      end
    end

    it "中断してもそのあと普通に変換できる" do
      Sghtmltopdf.render(multipage_html, chunk_size: 512) { |_| raise "止める" }
    rescue RuntimeError
      expect(Sghtmltopdf.render("<p>ok</p>")).to start_with("%PDF-")
    end
  end

  describe "他のスレッド" do
    it "レンダリング中もGVLが解放されている" do
      counter = 0
      # ビジーループにするとGVLを奪い合ってレンダリング自体が遅くなるので、
      # 少し眠りながら進める形で「他スレッドが動けたか」だけを見る。
      worker = Thread.new do
        loop do
          counter += 1
          sleep 0.001
        end
      end
      Sghtmltopdf.render(multipage_html, chunk_size: 512) { |_| }
      worker.kill
      worker.join

      expect(counter).to be > 0
    end
  end

  # Phase 6で得られた副次的な効果: ブロックの呼び出しはRubyのメソッド
  # 呼び出しなので、そこで保留中の割り込みが処理される。
  describe "割り込み" do
    # 変換そのものの速さはマシンによって数倍変わるので、「止めようとした時点で
    # まだ変換の途中」であることを実行時間に頼らずに作る。チャンクごとに少し
    # 眠らせれば、全体の所要時間は眠った時間の合計で決まる。
    CHUNK_SLEEP = 0.005

    it "Thread#killがチャンク境界で効く" do
      first_chunk = Queue.new
      thread = Thread.new do
        Sghtmltopdf.render(multipage_html(pages: 120), chunk_size: 512) do |_|
          first_chunk << true
          sleep CHUNK_SLEEP
        end
      end
      # 最初のチャンクが出るまで待ってから止める。
      first_chunk.pop
      thread.kill

      expect(thread.join(10)).to eq(thread)
      expect(thread.alive?).to be(false)
      # VMが壊れていないこと。
      expect(Sghtmltopdf.render("<p>ok</p>")).to start_with("%PDF-")
    end

    it "Timeout.timeoutが効く" do
      require "timeout"

      expect {
        Timeout.timeout(0.1) do
          Sghtmltopdf.render(multipage_html(pages: 120), chunk_size: 512) { |_| sleep CHUNK_SLEEP }
        end
      }.to raise_error(Timeout::Error)

      expect(Sghtmltopdf.render("<p>ok</p>")).to start_with("%PDF-")
    end
  end
end
