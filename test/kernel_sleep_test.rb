# frozen_string_literal: true

require_relative "test_helper"

class KernelSleepTest < Minitest::Test
  include GlobalSchedulerHooks

  def test_kernel_sleep_time
    called = false

    ret = Fiber.schedule do
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      10.times do
        Fiber.schedule { sleep(0.1) }
      end
      finish = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      called = true
      assert(finish - start >= 0.1, "Sleep time is less than expected, #{finish - start} < 0.1")
      assert(finish - start < 1, "Sleep time is more than expected, #{finish - start} > 1")
      :ok
    end.transfer

    assert_equal(:ok, ret)
    assert(called, "Fiber was not called")
  end

  def test_kernel_sleep_returns_int
    called = false

    Fiber.schedule do
      result = Fiber.scheduler.kernel_sleep(0.01)
      called = true
      assert_equal ::Kernel.sleep(0.01), result
    end

    assert(called, "Fiber was not called")
  end
end
