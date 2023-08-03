# frozen_string_literal: true

require "fiber_scheduler_spec/context"

module FiberSchedulerSpec
  module IOSelect
  end
end

RSpec.shared_examples FiberSchedulerSpec::IOSelect do
  describe "#io_select" do
    include_context FiberSchedulerSpec::Context

    context "without a timeout" do
      let(:order) { [] }
      let(:pair) { UNIXSocket.pair }
      let(:reader) { pair.first }
      let(:writer) { pair.last }

      def operations
        Fiber.schedule do
          order << 1
          IO.select([reader], [])
          order << 6
        end

        order << 2

        Fiber.schedule do
          order << 3
          writer.write(".")
          writer.close
          order << 4
        end
        order << 5
      end

      it "behaves async" do
        setup

        expect(order).to eq((1..6).to_a).or(eq([1, 2, 3, 5, 4, 6]))
      end

      it "calls #io_select" do
        expect_any_instance_of(scheduler_class)
          .to receive(:io_select).once
          .and_call_original

        setup
      end
    end

    context "with a timeout" do
      let(:order) { [] }
      let(:pair) { new_nonblock_unix_pair }
      let(:results) { [] }
      let(:reader) { pair.first }
      let(:writer) { pair.last }

      def operations
        reader.nonblock = true

        Fiber.schedule do
          order << 1
          results << IO.select([reader], [$stdout], nil, 0.1)
          order << 3
        end
        order << 2
      end

      it "behaves async" do
        setup

        expect(order).to eq (1..3).to_a
        expect(results).to eq [[[], [$stdout], []]]
      end

      it "calls #io_select" do
        expect_any_instance_of(scheduler_class)
          .to receive(:io_select).once
          .and_call_original

        setup
      end
    end
  end

  describe "#io_select using a real socket that cannot be read from" do
    it "behaves async" do
      order = []
      results = []
      read_socket, write_socket = new_nonblock_io_pipe

      TestHelpers.in_fibered_env do
        Fiber.schedule do
          order << 1
          results << IO.select([], [write_socket], nil, 1)
          write_socket.write_nonblock(".")
          results << IO.select([read_socket], [], nil, 1)
          order << 2
        end
      end

      expect(results[0]).to eq [[], [write_socket], []]
      expect(results[1]).to eq [[read_socket], [], []]
      expect(order).to eq [1, 2]
    end
  end
end
