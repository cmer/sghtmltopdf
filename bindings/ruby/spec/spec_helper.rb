# frozen_string_literal: true

require "sghtmltopdf"

# Ruby 4.0でbenchmarkが同梱されなくなったため、依存を増やさず自前で測る。
def elapsed_seconds
  started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  yield
  Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
end

# PDFの`/CreationDate`だけは実行時刻が入る(固定長なので他のオフセットには
# 影響しない)。バイト列を比べるときはここを固定してから比較する。
def normalize(pdf)
  pdf.gsub(/D:\d{14}Z/, "D:19700101000000Z")
end

RSpec.configure do |config|
  config.expect_with(:rspec) { |c| c.syntax = :expect }
  config.disable_monkey_patching!
  config.order = :random
  Kernel.srand config.seed
end
