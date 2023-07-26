use magnus::{
    block::Proc, exception::runtime_error, scan_args, typed_data::Obj, Error, ExceptionClass,
    IntoValue, RArray, RString, TryConvert, Value,
};
use std::os::fd::RawFd;

use crate::{
    fiber::{Fiber, Suspended},
    timeout_duration::TimeoutDuration,
};

bitflags::bitflags! {
  /// Bitflags struct for IO interests.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub struct IoInterests: u32 {
      const Readable = 0b00000001;
      const Writable = 0b00000010;
      const ReadableWritable = Self::Readable.bits() | Self::Writable.bits();
  }

}

impl TryConvert for IoInterests {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        let value: u32 = value.try_convert()?;

        IoInterests::from_bits(value).ok_or_else(|| {
            magnus::Error::new(runtime_error(), format!("Invalid IO interests: {}", value))
        })
    }
}

impl IntoValue for IoInterests {
    fn into_value_with(self, handle: &magnus::Ruby) -> Value {
        *handle.integer_from_u64(self.bits() as u64)
    }
}

impl From<IoInterests> for tokio::io::Interest {
    fn from(interests: IoInterests) -> Self {
        if interests.contains(IoInterests::ReadableWritable) {
            tokio::io::Interest::READABLE | tokio::io::Interest::WRITABLE
        } else if interests.contains(IoInterests::Readable) {
            tokio::io::Interest::READABLE
        } else if interests.contains(IoInterests::Writable) {
            tokio::io::Interest::WRITABLE
        } else {
            tokio::io::Interest::READABLE
        }
    }
}

/// Interface for a Ruby non-blocking Fiber scheduler.
///
/// ### Behavior
///
/// Scheduler’s behavior and usage are expected to be as follows:
///
/// - When the execution in the non-blocking Fiber reaches some blocking operation
///   (like sleep, wait for a process, or a non-ready I/O), it calls some of the
///   scheduler’s hook methods, listed below.
///
/// - Scheduler somehow registers what the current fiber is waiting on, and yields
///   control to other fibers with Fiber.yield (so the fiber would be suspended while
///   expecting its wait to end, and other fibers in the same thread can perform)
///
/// - At the end of the current thread execution, the scheduler’s method
///   scheduler_close is called
///
/// - The scheduler runs into a wait loop, checking all the blocked fibers (which it
///   has registered on hook calls) and resuming them when the awaited resource is
///   ready (e.g. I/O ready or sleep time elapsed).
pub trait Scheduler: Sized {
    /// Runs the scheduler until all fibers are completed.
    fn run(&self) -> Result<(), magnus::Error>;

    /// Resolves a hostname to a list of IP addresses, returning `None` if the
    /// hostname cannot be resolved.
    ///
    /// Returns a Future that resolves to a list of IP addresses.
    fn address_resolve(&self, hostname: RString) -> Result<Value, magnus::Error>;

    /// Invoked by methods like Thread.join, and by Mutex, to signify that
    /// current Fiber is blocked until further notice (e.g. unblock) or until
    /// timeout has elapsed.
    ///
    /// - `blocker` is what we are waiting on, informational only (for debugging and
    ///   logging). There are no guarantee about its value.
    ///
    /// - Expected to return boolean, specifying whether the blocking operation was successful or not.
    fn block(
        &self,
        blocker: Value,
        timeout: Option<TimeoutDuration>,
    ) -> Result<bool, magnus::Error>;

    /// Invoked to wake up Fiber previously blocked with block (for example,
    /// Mutex#lock calls block and Mutex#unlock calls unblock). The scheduler should
    /// use the fiber parameter to understand which fiber is unblocked.
    ///
    /// blocker is what was awaited for, but it is informational only (for debugging
    /// and logging), and it is not guaranteed to be the same value as the blocker
    /// for block
    fn unblock(&self, blocker: Value, fiber_to_wake: Value) -> Result<(), magnus::Error>;

    // /// Called when the current thread exits, allowing the scheduler to finalize
    // /// waiting fibers.
    // fn close(self);

    /// Schedules the given Proc to run in a separate non-blocking fiber.
    fn fiber(&self, args: &[Value]) -> Result<Fiber<Suspended>, Error> {
        let args = scan_args::scan_args::<(), (), (), (), (), Proc>(args)?;
        let block: Proc = args.block;
        Fiber::<Suspended>::spawn_nonblocking(block)
    }

    // /// Reads data from an IO object into a buffer at a specified offset.
    // fn io_pread(
    //     &self,
    //     io: RawFd,
    //     buffer: &mut [u8],
    //     from: usize,
    //     length: usize,
    //     offset: u64,
    // ) -> Result<usize, i32>;

    // /// Writes data to an IO object from a buffer at a specified offset.
    // fn io_pwrite(
    //     &self,
    //     io: RawFd,
    //     buffer: &[u8],
    //     from: usize,
    //     length: usize,
    //     offset: u64,
    // ) -> Result<usize, i32>;

    // /// Invoked by IO#read to read length bytes from io into a specified buffer
    // /// (see IO::Buffer).
    // ///
    // /// The length argument is the “minimum length to be read”. If the IO buffer
    // /// size is 8KiB, but the length is 1024 (1KiB), up to 8KiB might be read,
    // /// but at least 1KiB will be. Generally, the only case where less data than
    // /// length will be read is if there is an error reading the data.
    // ///
    // /// Specifying a length of 0 is valid and means try reading at least once
    // /// and return any available data.
    // ///
    // /// Suggested implementation should try to read from io in a non-blocking
    // /// manner and call io_wait if the io is not ready (which will yield control
    // /// to other fibers).
    // ///
    // /// See IO::Buffer for an interface available to return data.
    // ///
    // /// Expected to return number of bytes read, or, in case of an error, -errno
    // /// (negated number corresponding to system’s error code).
    // ///
    // /// The method should be considered experimental.
    // fn io_read(
    //     &self,
    //     io: RawFd,
    //     buffer: &mut [u8],
    //     minimum_length_to_read: usize,
    // ) -> Result<usize, magnus::Error>;

    // /// Checks whether the specified IO objects are ready for the specified events within the given timeout.
    // fn io_select(
    //     &self,
    //     readables: RArray,
    //     writables: RArray,
    //     exceptables: RArray,
    //     timeout: Option<TimeoutDuration>,
    // ) -> Result<(RArray, RArray, RArray), magnus::Error>;

    /// Invoked by IO#wait, IO#wait_readable, IO#wait_writable to ask whether
    /// the specified descriptor is ready for specified events within the
    /// specified timeout.
    ///
    /// `events` is a bit mask of IO::READABLE, IO::WRITABLE, and IO::PRIORITY.
    ///
    /// Suggested implementation should register which Fiber is waiting for
    /// which resources and immediately calling Fiber.yield to pass control to
    /// other fibers. Then, in the close method, the scheduler might dispatch
    /// all the I/O resources to fibers waiting for it.
    ///
    /// Expected to return the subset of events that are ready immediately.
    fn io_wait(
        &self,
        io: RawFd,
        interests: IoInterests,
        timeout: Option<TimeoutDuration>,
    ) -> Result<Value, magnus::Error>;

    // /// Writes data to an IO object from a buffer.
    // fn io_write(&self, io: RawFd, buffer: &[u8], length: usize) -> Result<usize, i32>;

    /// Puts the current fiber to sleep for the specified duration.
    fn kernel_sleep(&self, duration: TimeoutDuration) -> Result<Value, magnus::Error>;

    /// Called when the current thread exits. The scheduler is expected to
    /// implement this method in order to allow all waiting fibers to finalize
    /// their execution.
    ///
    /// The suggested pattern is to implement the main event loop in the close method.
    fn close(rb_self: Obj<Self>) -> Result<(), magnus::Error>;

    /// Waits for the specified process with the given flags.
    fn process_wait(&self, pid: u32, flags: i32) -> Result<Value, magnus::Error>;

    /// Executes a given block within the specified duration, raising an
    /// exception if the block's execution time exceeds the duration.
    fn timeout_after(
        &self,
        duration: TimeoutDuration,
        exception_class: ExceptionClass,
        message: RString,
    ) -> Result<Value, magnus::Error>;
}

#[macro_export]
macro_rules! define_scheduler_methods {
    ($klass:ident) => {{
        use magnus::{method, Module};

        let c = $klass::class();

        c.define_singleton_method("new", function!($klass::new, 0))?;
        c.define_method("address_resolve", method!($klass::address_resolve, 1))?;
        c.define_method("io_wait", method!($klass::io_wait, 3))?;
        c.define_method("kernel_sleep", method!($klass::kernel_sleep, 1))?;
        c.define_method("close", method!($klass::close, 0))?;
        c.define_method("fiber", method!($klass::fiber, -1))?;
        c.define_method("timeout_after", method!($klass::timeout_after, 3))?;
        c.define_method("process_wait", method!($klass::process_wait, 2))?;
        c.define_method("block", method!($klass::block, 2))?;
        c.define_method("unblock", method!($klass::unblock, 2))?;
        c.define_method("run", method!($klass::run, 0))?;
    }};
}

#[macro_export]
macro_rules! impl_typed_data_for_scheduler {
    ($ident:ident) => {
        // TODO: remove this by implementing a proper `Send` trait for `TaskList`
        unsafe impl Send for $ident {}
        unsafe impl Sync for $ident {}

        unsafe impl TypedData for $ident {
            fn class() -> RClass {
                *memoize!(RClass: {
                  let c = magnus::define_class(stringify!($ident), Default::default()).unwrap();
                  c.undef_alloc_func();
                  c
                })
            }

            fn data_type() -> &'static DataType {
                memoize!(DataType: {
                  let mut builder = DataTypeBuilder::<Self>::new(stringify!($ident));
                  builder.mark();
                  builder.free_immediately();
                  builder.build()
                })
            }
        }
    };
}
