require "spec_helper"
require "resolv"

RSpec.describe SCHEDULER_IMPLEMENTATION do
  around do |example|
    result = Thread.new do
      scheduler = described_class.new
      Fiber.set_scheduler(scheduler)
      example.run
    end.join(1)

    expect(result).to be_a Thread # failure means spec timed out
  end

  cases = [
    "localhost",
    "localhost:3000",
    "localhost:80",
    "localhost%en0",
    "localhost%en0:3000",
    "example.com"
  ].freeze

  cases.each do |host|
    fit "can resolve #{host}" do
      called = false

      Fiber.schedule do
        parts = host.split(":", 2)
        result = if parts.length == 1
                   Fiber.scheduler.address_resolve(host)
                 else
                   Addrinfo.getaddrinfo(*parts).to_s
                 end
        called = true
        expected = Resolv.getaddresses(host.split("%", 2).first)

        expected.each do |expected_item|
          expect(result).to include(expected_item)
        end

        expect(result.length).to be >= expected.length
      end

      Fiber.scheduler.run
      expect(called).to be(true), "Fiber was not called"
    end
  end
end
