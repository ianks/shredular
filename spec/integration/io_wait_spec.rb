# frozen_string_literal: true

require "spec_helper"
require "resolv"

RSpec.describe "#io_wait" do
  around do |example|
    TestHelpers.in_fibered_env { example.run }
  end

  let(:pair) { UNIXSocket.pair }
  let(:reader) { pair.first }
  let(:writer) { pair.last }

  it "does not immediately raise when waiting readable" do
    Fiber.schedule do
      expect { reader.wait_readable(0.001) }.not_to raise_error
    end

    Fiber.scheduler.run
  end

  it "does not immediately raise when waiting writable" do
    Fiber.schedule do
      expect { reader.wait_writable(0.001) }.not_to raise_error
    end

    Fiber.scheduler.run
  end
end
