# frozen_string_literal: true

require "sghtmltopdf"

Dir[File.join(__dir__, "support", "**", "*.rb")].sort.each { |file| require file }

# Ruby 4.0でbenchmarkが同梱されなくなったため、依存を増やさず自前で測る。
def elapsed_seconds
  started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  yield
  Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
end

# PDFの`/CreationDate`とtrailerの`/ID`には実行時刻が入る(どちらも固定長
# なので他のオフセットには影響しない)。バイト列を比べるときはここを固定して
# から比較する。
def normalize(pdf)
  pdf.gsub(/D:\d{14}Z/, "D:19700101000000Z")
     .gsub(/\/ID \[<\h{32}> <\h{32}>\]/, "/ID [<#{'0' * 32}> <#{'0' * 32}>]")
end

RSpec.configure do |config|
  config.expect_with(:rspec) { |c| c.syntax = :expect }
  config.disable_monkey_patching!
  config.order = :random
  Kernel.srand config.seed
end
