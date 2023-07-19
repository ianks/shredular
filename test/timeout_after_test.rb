# frozen_string_literal: true

require "test_helper"

class TimeoutAfterTest < Minitest::Test
  include GlobalSchedulerHooks

  def test_timeout_after_time
    called = false
    my_error = Class.new(StandardError)

    start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    Fiber.schedule do
      Fiber.scheduler.timeout_after(0.5, my_error, ["My message"]) do
        called = true
        loop do
          scheduler.kernel_sleep(0)
        end
      end
      finish = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      called = true
      assert(finish - start >= 0.5, "Sleep time is less than expected, #{finish - start} < 0.5")
      assert(finish - start < 1, "Sleep time is more than expected, #{finish - start} > 1")
    end

    assert(called, "Fiber was not called")
  end
end
