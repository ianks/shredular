# frozen_string_literal: true

require "test_helper"

class FiberTest < Minitest::Test
  def test_fiber_spawns_a_nonblocking_fiber
    called = false

    with_scheduler(global: false) do |scheduler|
      scheduler.fiber do
        called = true
        refute(Fiber.current.blocking?)
      end
    end

    assert(called, "Fiber was not called")
  end
end
