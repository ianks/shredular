# frozen_string_literal: true

require "mkmf"
require "rb_sys/mkmf"

create_rust_makefile("shreduler/shreduler") do |ext|
  ext.extra_rustflags << "--cfg tokio_unstable"
end
