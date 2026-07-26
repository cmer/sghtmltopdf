# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

# `lib/sghtmltopdf/sghtmltopdf.so`として作る。
# `Init_sghtmltopdf`はCargo.tomlのパッケージ名から生成される
# (docs/decisions/0062-ruby-binding.md「パッケージ名がInit_シンボル名を決める」)。
create_rust_makefile("sghtmltopdf/sghtmltopdf")
