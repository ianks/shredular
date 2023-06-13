# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path("../lib", __dir__)

require "shreduler"
require "minitest/autorun"

module Minitest
  class Test
    if ENV["GC_STRESS"]
      def setup
        GC.stress = true
        super
      end

      def teardown
        GC.stress = false
        super
      end
    end

    def with_scheduler
      @scheduler = TokioScheduler.new
      Fiber.set_scheduler(@scheduler)
      yield
    ensure
      @scheduler.shutdown
      Fiber.set_scheduler(nil)
    end

    def spawn_fiber(blocking: true, &block)
      fiber = Fiber.new(blocking: blocking, &block)
      fiber.resume
      fiber
    end
  end
end
