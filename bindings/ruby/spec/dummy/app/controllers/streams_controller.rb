# frozen_string_literal: true

# 確定したページから順にRackのレスポンスへ流す(T355)。
#
# `render pdf:`のレンダラは組み上がったPDFを`send_data`で一括返却するので、
# 逐次で返したい場合は`ActionController::Live`とブロック付き`render`を
# 直接組み合わせる(docs/decisions/0063-ffi-chunk-callback.md)。
class StreamsController < ActionController::Base
  include ActionController::Live

  def show
    response.headers["Content-Type"] = "application/pdf"
    html = render_to_string(template: "invoices/long", layout: false)
    Sghtmltopdf.render(html, chunk_size: 1024) { |bytes| response.stream.write(bytes) }
  ensure
    response.stream.close
  end
end
