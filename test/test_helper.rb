# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path("../lib", __dir__)

require "shreduler"
require "maxitest/autorun"

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

    def with_scheduler(global:)
      raise ArgumentError, "block is required" unless block_given?

      @scheduler = Shredular.new
      @scheduler.close unless global
      Fiber.set_scheduler(@scheduler) if global
      yield(@scheduler)
    ensure
      @scheduler.shutdown
      Fiber.set_scheduler(nil) if global
    end
  end
end
