# frozen_string_literal: true

require "spec_helper"
require "resolv"

RSpec.describe SCHEDULER_IMPLEMENTATION do
  describe "#io_read and #io_write" do
    it "behaves async" do
      order = []
      results = []
      read_socket, write_socket = IO.pipe

      TestHelpers.in_fibered_env do
        Fiber.schedule do
          order << 1
          results << read_socket.read(4)
          order << 4
        end

        Fiber.schedule do
          order << 2
          write_socket.write("ruby")
          order << 3
        end
      end

      expect(order).to eq [1, 2, 3, 4]
      expect(results).to eq ["ruby"]
    end
  end
end
