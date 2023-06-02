use magnus::{exception::runtime_error, IntoValue, RArray, TryConvert, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    future::{Future, IntoFuture},
    os::fd::RawFd,
    task::{Context, Poll},
};
use tokio::io::unix::AsyncFd;

use crate::{
    fiber::{Fiber, Running, State, Suspended},
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

unsafe impl Send for IoInterests {}

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
pub trait Scheduler {
    /// Resolves a hostname to a list of IP addresses, returning `None` if the
    /// hostname cannot be resolved.
    ///
    /// Returns a Future that resolves to a list of IP addresses.
    fn address_resolve(&self, hostname: String) -> Result<Value, magnus::Error>;

    // /// Invoked by methods like Thread.join, and by Mutex, to signify that
    // /// current Fiber is blocked until further notice (e.g. unblock) or until
    // /// timeout has elapsed.
    // ///
    // /// - `blocker` is what we are waiting on, informational only (for debugging and
    // ///   logging). There are no guarantee about its value.
    // ///
    // /// - Expected to return boolean, specifying whether the blocking operation was successful or not.
    // fn block(
    //     &self,
    //     blocker: Value,
    //     timeout: Option<TimeoutDuration>,
    // ) -> Pin<Box<dyn Future<Output = Result<bool, magnus::Error>>>>;

    // /// Called when the current thread exits, allowing the scheduler to finalize
    // /// waiting fibers.
    // fn close(self);

    // /// Schedules the given Proc to run in a separate non-blocking fiber.
    // fn fiber(&self, block: Proc) -> Result<(), magnus::Error>;

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

    // /// Puts the current fiber to sleep for the specified duration.
    // fn kernel_sleep(&self, duration: Option<TimeoutDuration>);

    // /// Waits for the specified process with the given flags.
    // fn process_wait(&self, pid: u32, flags: i32) -> Result<Value, i32>;

    // /// Executes a given block within the specified duration, raising an
    // /// exception if the block's execution time exceeds the duration.
    // fn timeout_after<F: FnOnce() -> T, T>(
    //     &self,
    //     duration: TimeoutDuration,
    //     exception_class: Value,
    //     exception_arguments: RArray,
    //     block: F,
    // ) -> Result<T, magnus::Error>;
}

pub type WaitList = BTreeMap<
    Fiber<Suspended>,
    tokio::task::JoinHandle<std::result::Result<magnus::Value, magnus::Error>>,
>;

pub struct TokioScheduler<S: State> {
    root_fiber: Fiber<S>,
    runtime: tokio::runtime::Runtime,
    future_fibers: RefCell<WaitList>,
}

impl TokioScheduler<Running> {
    /// Implementation of the `Scheduler` interface for the Tokio runtime.
    ///
    /// Internally, this scheduler uses the `tokio::runtime::Runtime` execute and
    /// poll futures: yielding control to other fibers when a future is not ready,
    /// and resuming the fiber when the future is ready.
    /// Creates a new Tokio scheduler.
    pub fn new() -> Result<Self, magnus::Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| magnus::Error::new(runtime_error(), format!("{e}")))?;

        Ok(Self {
            root_fiber: Fiber::current(),
            runtime,
            future_fibers: Default::default(),
        })
    }

    pub fn run_event_loop(&self) -> Result<(), magnus::Error> {
        self.runtime
            .block_on(async { tokio::signal::ctrl_c().await })
            .map_err(|e: std::io::Error| magnus::Error::new(runtime_error(), format!("{e}")))
    }
}

impl<S: State> TokioScheduler<S> {
    pub fn root_fiber(&self) -> &Fiber<S> {
        &self.root_fiber
    }
}

impl TokioScheduler<Suspended> {
    pub fn register_fiber<S: State, F: Future<Output = Result<Value, magnus::Error>> + 'static>(
        &self,
        fiber: Fiber<S>,
        future: F,
    ) -> Fiber<Suspended> {
        let fiber = unsafe { fiber.as_suspended() };
        let mut future_fibers = self.future_fibers.borrow_mut();
        // SAFETY: The fiber is actually running, but we need to save it as
        // suspended and decrement the reference count befor actually suspending
        // it.
        /// locally spawned futures are not Send, so we need to spawn them on the
        /// runtime.
        let pinned = Box::pin(future);

        let local_task = tokio::task::spawn_local(pinned);

        future_fibers.insert(fiber, self.runtime.spawn(Box::pin(future)));
        fiber
    }

    pub fn poll_fiber_result_in_loop(
        &self,
        fiber: Fiber<Suspended>,
    ) -> Result<Value, magnus::Error> {
        let mut future_fibers = self.future_fibers.borrow_mut();

        loop {
            let Some(handle) = future_fibers.get_mut(&fiber) else {
                return Err(magnus::Error::new(
                    runtime_error(),
                    "fiber not found in future_fibers"
                ));
            };

            tokio::pin!(handle);

            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            match handle.poll(&mut cx) {
                Poll::Ready(Ok(Ok(result))) => {
                    future_fibers.remove(&fiber).expect("fiber should exist");
                    fiber.transfer([result])?;
                }
                Poll::Ready(Ok(Err(e))) => {
                    future_fibers.remove(&fiber).expect("fiber should exist");
                    fiber.raise(runtime_error(), format!("{e}"))?;
                    return Err(e);
                }
                Poll::Ready(Err(e)) => {
                    future_fibers.remove(&fiber).expect("fiber should exist");
                    fiber.raise(runtime_error(), format!("{e}"))?;
                    return Err(magnus::Error::new(runtime_error(), format!("{e}")));
                }
                Poll::Pending => {
                    self.root_fiber().transfer(())?;
                    continue;
                }
            };
        }
    }
}

impl Scheduler for TokioScheduler<Suspended> {
    fn address_resolve(&self, hostname: String) -> Result<Value, magnus::Error> {
        let future = async move {
            let mut addresses = Vec::new();

            let host_lookup = tokio::net::lookup_host(&hostname).await;
            let host_lookup =
                host_lookup.map_err(|e| magnus::Error::new(runtime_error(), format!("{e}")));

            for address in host_lookup? {
                addresses.push(address.to_string());
            }

            Ok(*RArray::from_vec(addresses))
        };

        let wait_fiber = self.register_fiber(Fiber::current(), future);
        self.root_fiber().transfer(())?;
        self.poll_fiber_result_in_loop(wait_fiber)
    }

    fn io_wait(
        &self,
        io: RawFd,
        interests: IoInterests,
        timeout: Option<TimeoutDuration>,
    ) -> Result<Value, magnus::Error> {
        let base_future = async move {
            let tokio_interests = tokio::io::Interest::from(interests);
            let async_fd = AsyncFd::with_interest(io, tokio_interests).map_err(|e| {
                magnus::Error::new(runtime_error(), format!("Failed to create AsyncFd: {e}"))
            })?;
            if interests.contains(IoInterests::Readable) {
                let _ = async_fd.readable().await.map_err(|e| {
                    magnus::Error::new(runtime_error(), format!("Failed to wait for readable: {e}"))
                })?;
            }

            if interests.contains(IoInterests::Writable) {
                let _ = async_fd.writable().await.map_err(|e| {
                    magnus::Error::new(runtime_error(), format!("Failed to wait for writable: {e}"))
                })?;
            }

            Ok(interests.into_value())
        };

        let future_with_timeout = async move {
            let timeout = timeout.unwrap_or_default();

            tokio::select! {
              result = base_future => {
                result
              }
              _ = timeout.into_future() => {
                Err(magnus::Error::new(
                  runtime_error(),
                  format!("Timeout after {}", timeout),
                ))
              }
            }
        };

        let wait_fiber = self.register_fiber(Fiber::current(), future_with_timeout);
        self.root_fiber().transfer(())?;
        self.poll_fiber_result_in_loop(wait_fiber)
    }
}
