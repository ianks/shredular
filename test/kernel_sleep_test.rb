# frozen_string_literal: true

require "test_helper"

class KernelSleepTest < Minitest::Test
  def test_kernel_sleep_time
    called = false

    with_scheduler(global: false) do |scheduler|
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      scheduler.kernel_sleep(0.1)
      finish = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      called = true
      assert(finish - start >= 0.1, "Sleep time is less than expected, #{finish - start} < 0.1")
    end

    assert(called, "Fiber was not called")
  end

  def test_kernel_sleep_returns_int
    called = false

    with_scheduler(global: false) do |scheduler|
      result = scheduler.kernel_sleep(0.01)
      called = true
      assert_equal ::Kernel.sleep(0.01), result
    end

    assert(called, "Fiber was not called")
  end
end
