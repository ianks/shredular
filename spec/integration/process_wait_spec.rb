# frozen_string_literal: true

require "spec_helper"
require "resolv"

RSpec.describe SCHEDULER_IMPLEMENTATION do
  describe "#process_wait" do
    around do |example|
      TestHelpers.in_fibered_env { example.run }
    end

    it "bubbles up the errno" do
      Fiber.schedule do
        result = Fiber.scheduler.process_wait(999_999, 0)
        expect(result.pid).to eq(-1)
        expect(result.to_i).to eq(0)

        expect { Process.wait(999_999) }.to raise_error(Errno::ECHILD)
      end

      Fiber.scheduler.run
    end
  end
end
