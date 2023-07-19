# frozen_string_literal: true

require_relative "test_helper"
require "resolv"

class AddressResolveTest < Minitest::Test
  include GlobalSchedulerHooks

  CASES = [
    "localhost",
    "localhost:3000",
    "localhost:80",
    "localhost%en0",
    "localhost%en0:3000",
    "example.com"
  ].freeze

  CASES.each do |host|
    slug = host.gsub(/[^a-z0-9]+/i, "_")
    define_method("test_address_resolve_parity_#{slug}") do
      called = false

      Fiber.schedule do
        5.times do
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
              assert_includes(result, expected_item)
            end

            assert(result.length >= expected.length)
          end
        end
      end

      assert(called, "Fiber was not called")
    end
  end
end
