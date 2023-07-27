# frozen_string_literal: true

require "spec_helper"
require "resolv"

RSpec.describe SCHEDULER_IMPLEMENTATION do
  around do |example|
    TestHelpers.in_fibered_env { example.run }
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
    it "can resolve #{host}" do
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
