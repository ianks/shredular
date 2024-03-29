# frozen_string_literal: true

require "bundler/gem_tasks"
require "rubocop/rake_task"
require "rb_sys/extensiontask"
require "rspec/core/rake_task"

RSpec::Core::RakeTask.new(:spec)

RuboCop::RakeTask.new(:rubocop)

desc "Run clippy"
task :clippy do
  sh("cargo clippy --all-targets --all-features -- -D warnings")
end

RbSys::ExtensionTask.new("shreduler") do |ext|
  ext.lib_dir = "lib/shreduler"
end

namespace :test do
  desc "Run tests with async scheduler"
  task :async do
    ENV["SCHEDULER_IMPLEMENTATION"] = "async"
    sh("bundle exec rake spec")
  end

  %i[trace debug info warn error fatal].each do |level|
    desc "Run tests with #{level} log level"
    task level do
      ENV["RUST_LOG"] = "shreduler=#{level}"
      sh("bundle exec rake")
    end
  end

  desc "Run cargo test"
  task :cargo do
    sh("cargo test")
  end
end

desc "Run tests"
task test: ["test:cargo", "spec"]

desc "Run lints"
task lint: %i[rubocop clippy]

task default: %i[compile test]
