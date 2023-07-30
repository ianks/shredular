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
          IO.select([reader], [$stdout])
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

        expect(order).to eq (1..6).to_a
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
      let(:pair) { UNIXSocket.pair }
      let(:reader) { pair.first }
      let(:writer) { pair.last }

      def operations
        Fiber.schedule do
          order << 1
          IO.select([reader], [$stdout], nil, 0.1)
          order << 3
        end
        order << 2
      end

      it "behaves async" do
        setup

        expect(order).to eq (1..3).to_a
      end

      it "calls #io_select" do
        expect_any_instance_of(scheduler_class)
          .to receive(:io_select).once
          .and_call_original

        setup
      end
    end
  end

  describe "using a real socket that cannot be read from" do
    around do |example|
      TestHelpers.in_fibered_env { example.run }
    end

    it "behaves async" do
      read_socket, write_socket = UNIXSocket.pair
      order = []
      results = []

      Fiber.schedule do
        order << 1
        results << IO.select([read_socket], [write_socket], nil, 5)
        order << 2
      end

      expect(results).to eq [[[], [write_socket], []]]
      expect(order).to eq [1, 2]
    end
  end
end
