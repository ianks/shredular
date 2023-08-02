# frozen_string_literal: true

require "rspec"

module FiberSchedulerSpec
  module Context
  end
end

RSpec.shared_context FiberSchedulerSpec::Context do
  let(:scheduler_class) { described_class } unless method_defined?(:scheduler_class)
  subject(:scheduler) { scheduler_class.new } unless method_defined?(:scheduler)
  def setup
    Fiber.set_scheduler(scheduler)

    operations

    scheduler.run
  end

  around do |example|
    result = Thread.new do
      example.run
    end.join

    expect(result).to be_a Thread # failure means spec timed out
  end
end
