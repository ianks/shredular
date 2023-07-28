use super::prelude::*;
use bitflags::bitflags;
use magnus::value::Id;
use magnus::{IntoValue, TryConvert};
use rb_sys::rb_io_event_t::*;
use std::convert::TryFrom;
use std::os::fd::RawFd;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;

use super::base_error;

impl TokioScheduler {
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
    #[tracing::instrument]
    pub fn io_wait(
        &self,
        io: Value,
        interests: RubyIoEvent,
        timeout: Option<TimeoutDuration>,
    ) -> Result<Value, magnus::Error> {
        let fileno = *memoize!(Id: Id::new("fileno"));
        let ruby_io: RawFd = io.funcall(fileno, ())?;
        let interests = tokio::io::Interest::try_from(interests)?;

        let future = async move {
            let async_fd = AsyncFd::with_interest(ruby_io, interests).map_err(|e| {
                magnus::Error::new(
                    base_error(),
                    format!("Could not create AsyncFd from RawFd: {}", e),
                )
            })?;
            let _ = async_fd.ready(interests).await.map_err(|e| {
                magnus::Error::new(base_error(), format!("Could not wait for readable: {}", e))
            })?;

            Ok(RubyIoEvent::READABLE.into_value())
        };

        let future = Self::with_timeout_no_raise(timeout, future);
        self.spawn_and_transfer(future)
    }
}

bitflags! {
  /// Bitflags struct for IO interests.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub struct RubyIoEvent: u32 {
      const READABLE = RUBY_IO_READABLE as _;
      const WRITABLE = RUBY_IO_WRITABLE as _;
      const PRIORITY = RUBY_IO_PRIORITY as _;
  }

}

impl TryConvert for RubyIoEvent {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        let value: u32 = value.try_convert()?;

        RubyIoEvent::from_bits(value).ok_or_else(|| {
            magnus::Error::new(base_error(), format!("Invalid IO interests: {}", value))
        })
    }
}

impl IntoValue for RubyIoEvent {
    fn into_value_with(self, handle: &magnus::Ruby) -> Value {
        *handle.integer_from_u64(self.bits() as u64)
    }
}

impl TryFrom<RubyIoEvent> for Interest {
    type Error = Error;

    fn try_from(interests: RubyIoEvent) -> Result<Self, Self::Error> {
        let result = if interests.contains(RubyIoEvent::READABLE) {
            Interest::READABLE
        } else {
            Interest::WRITABLE
        };

        if interests.contains(RubyIoEvent::WRITABLE) {
            result.add(Interest::WRITABLE);
        }

        if interests.contains(RubyIoEvent::PRIORITY) {
            return Err(Error::new(
                base_error(),
                "Priority is not a valid interest for tokio::io::Interest",
            ));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_sys_test_helpers::ruby_test;

    #[ruby_test]
    fn test_conversions_from_ruby_io_interests() {
        let readable: RubyIoEvent = eval!("IO::READABLE").unwrap();
        assert_eq!(tokio::io::Interest::READABLE, readable.try_into().unwrap());

        let writable: RubyIoEvent = eval!("IO::WRITABLE").unwrap();
        assert_eq!(tokio::io::Interest::WRITABLE, writable.try_into().unwrap());

        #[cfg(target_os = "linux")]
        {
            let priority: RubyIoInterests = eval!("IO::PRIORITY").unwrap();
            assert_eq!(tokio::io::Ready::PRIORITY, writable.into());
        }
    }
}
