# frozen_string_literal: true

require "spec_helper"
require "resolv"

RSpec.describe "#io_read and #io_write" do
  it "behaves async" do
    order = []
    results = []
    read_socket, write_socket = new_nonblock_unix_pair

    TestHelpers.in_fibered_env do
      Fiber.schedule do
        order << 1
        results << read_socket.read(4)
        order << 2
      end

      Fiber.schedule do
        order << 3
        write_socket.write("ruby")
        order << 4
      end
    end

    expect(order).to eq [1, 3, 2, 4]
    expect(results).to eq ["ruby"]
  end
end
