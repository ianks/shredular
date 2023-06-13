# frozen_string_literal: true

require "test_helper"

class TestShreduler < Minitest::Test
  def xtest_address_resolve_basic
    scheduler = Shredular.new

    spawn_fiber(blocking: false) do
      result = scheduler.address_resolve("localhost:3000")
      assert_equal ["[::1]:3000", "127.0.0.1:3000"], result
    end

    scheduler.shutdown
  end

  def test_kernel_sleep_time
    scheduler = Shredular.new

    spawn_fiber(blocking: false) do
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      scheduler.kernel_sleep(0.01)
      finish = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      puts "Sleep time: #{finish - start}"
      assert finish - start >= 0.01, "Sleep time is less than expected, #{finish - start} < 0.01"
    end

    scheduler.shutdown
  end
end
