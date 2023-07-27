use std::os::fd::RawFd;

use crate::rtodo;

use super::prelude::*;
use magnus::{IntoValue, TryConvert};

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
        io: RawFd,
        interests: IoInterests,
        timeout: Option<TimeoutDuration>,
    ) -> Result<Value, magnus::Error> {
        rtodo!("io_wait")
    }
}

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
            magnus::Error::new(base_error(), format!("Invalid IO interests: {}", value))
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
