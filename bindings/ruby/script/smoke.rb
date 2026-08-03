# frozen_string_literal: true

# インストール済みのgemが実際に動くかだけを見る最小の確認
# precompiled gemを`gem install`したあと、リポジトリのlibを使わずに
# 実行すること(`bundle exec`や`-Ilib`を付けない)。

require "sghtmltopdf"

pdf = Sghtmltopdf.render(<<~HTML, page_size: "A4", margin_top: "20mm")
  <html><head><title>smoke</title></head>
  <body><h1>sghtmltopdf</h1><p>precompiled gem smoke test</p></body></html>
HTML

abort "PDFになっていません: #{pdf[0, 20].inspect}" unless pdf.start_with?("%PDF-")
abort "PDFが終端していません" unless pdf.end_with?("%%EOF")
abort "PDFが小さすぎます: #{pdf.bytesize}バイト" if pdf.bytesize < 500

require "tmpdir"
Dir.mktmpdir do |dir|
  path = File.join(dir, "smoke.pdf")
  Sghtmltopdf.render_to_file("<p>file</p>", path)
  abort "ファイルへ書き出せていません" unless File.binread(path).start_with?("%PDF-")
end

puts "ok: sghtmltopdf #{Sghtmltopdf::VERSION} / ruby #{RUBY_VERSION} #{RUBY_PLATFORM} / #{pdf.bytesize} bytes"
