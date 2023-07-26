# frozen_string_literal: true

require "bundler/gem_tasks"
require "rubocop/rake_task"
require "rb_sys/extensiontask"
require "rspec/core/rake_task"

RSpec::Core::RakeTask.new(:test)

RuboCop::RakeTask.new

RbSys::ExtensionTask.new("shreduler") do |ext|
  ext.lib_dir = "lib/shreduler"
end

namespace :test do
  desc "Run tests with async scheduler"
  task :async do
    ENV["SCHEDULER_IMPLEMENTATION"] = "async"
    sh("bundle exec rake test")
  end
end

task default: %i[compile test]
