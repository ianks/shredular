# frozen_string_literal: true

require_relative "test_helper"

class FiberTest < Minitest::Test
  include GlobalSchedulerHooks

  def test_fiber_spawns_a_nonblocking_fiber
    called = false

    Fiber.schedule do
      called = true
      refute(Fiber.current.blocking?)
    end

    assert(called, "Fiber was not called")
  end
end
