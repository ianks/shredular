# frozen_string_literal: true

require "test_helper"
require "resolv"

class AddressResolveTest < Minitest::Test
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

      with_scheduler(global: false) do |scheduler|
        scheduler.fiber do
          5.times do
            scheduler.fiber do
              result = scheduler.address_resolve(host)
              called = true
              expected = Resolv.getaddresses(host.split("%", 2).first)

              expected.each do |expected_item|
                assert_includes(result, expected_item)
              end

              assert(result.length >= expected.length)
            end
          end
        end
      end

      assert(called, "Fiber was not called")
    end
  end
end
