# frozen_string_literal: true

RSpec.describe Sghtmltopdf do
  it "バージョンを持つ" do
    expect(Sghtmltopdf::VERSION).to match(/\A\d+\.\d+\.\d+\z/)
  end

  describe "ネイティブ拡張" do
    it "coreへリンクできている" do
      # coreのPageSettings::default()を呼んだ結果(A4のpxサイズ)。
      expect(Sghtmltopdf::Native.default_page_size).to eq("793.7x1122.5")
    end

    it "GVLを解放するので他のスレッドが並行して進む" do
      # 4スレッド×300ms。GVLを握ったままなら約1200msかかる
      # (docs/decisions/0062-ruby-binding.md 決定6)。
      elapsed = elapsed_seconds do
        4.times.map { Thread.new { Sghtmltopdf::Native.sleep_without_gvl(300) } }.each(&:join)
      end
      expect(elapsed).to be < 0.9
    end
  end
end
