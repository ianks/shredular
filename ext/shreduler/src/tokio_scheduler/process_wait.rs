use std::{
    ffi::c_void,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;
use magnus::{
    class::object,
    gc::register_mark_object,
    rb_sys::{protect, AsRawValue},
    Module, RClass, RModule, QNIL,
};
use rustix::process::{waitpid, Pid, RawPid, WaitOptions};
use tokio::signal::unix::{self, SignalKind};
use tracing::warn;

use crate::new_base_error;

use super::prelude::*;

impl TokioScheduler {
    /// Waits for the specified process with the given flags.
    #[tracing::instrument]
    pub fn process_wait(&self, pid: i32, flags: u32) -> Result<Value, Error> {
        let future = async move {
            let waiter = PidWaiter::new(pid, flags)?;

            waiter.await
        };

        self.spawn_and_transfer(future)
    }
}

pub struct PidWaiter {
    pid: i32,
    flags: u32,
    signal: tokio::signal::unix::Signal,
}

impl PidWaiter {
    pub fn new(pid: i32, flags: u32) -> Result<Self, Error> {
        let signal = unix::signal(SignalKind::child())
            .map_err(|e| new_base_error!("failed to create signal watcher: {e}"))?;

        Ok(Self { pid, flags, signal })
    }

    fn try_wait(&self) -> Result<Option<ProcessStatus>, Error> {
        let raw_pid = self.pid;
        let flags = self.flags;

        let Some(options) = WaitOptions::from_bits(flags) else {
          return Err(
            new_base_error!("invalid process wait flags: {flags}"),
          );
        };

        // > 0  | Waits for the child whose process ID equals pid.
        // 0    | Waits for any child whose process group ID equals that of the calling process.
        // -1   | Waits for any child process (the default if no pid is given).
        // < -1 | Waits for any child whose process group ID equals the absolute value of pid.
        #[allow(clippy::if_same_then_else)]
        let pid = if raw_pid == 0 {
            None
        } else if raw_pid == -1 {
            Some(Pid::from_raw(RawPid::MAX).unwrap())
        } else if raw_pid.wrapping_neg() < raw_pid {
            Some(Pid::from_raw(raw_pid).unwrap())
        } else if raw_pid > 0 {
            Some(Pid::from_raw(raw_pid).unwrap())
        } else {
            return Err(new_base_error!("Invalid PID for process: {raw_pid}"));
        };

        match waitpid(pid, options) {
            Ok(None) => Ok(None),
            Err(e) => {
                // This looks strange, but it's what MRI does
                let errno = e.raw_os_error();
                let status = 0;
                let pid = -1;
                ProcessStatus::new(pid, status, errno).map(Some)
            }
            Ok(Some(s)) => {
                let pid = raw_pid;
                let status = s.as_raw() as i32;

                ProcessStatus::new(pid, status, 0).map(Some)
            }
        }
    }
}

/// A wrapper around the `Process::Status` class.
///
/// Unfortunately, we can't use the `Process::Status` class directly because
/// there is no way to initialize it with our own data, so we wrap it in a
/// struct that we can allocate and initialize ourselves.
#[derive(Clone, Debug)]
struct ProcessStatus(magnus::RTypedData);

impl ProcessStatus {
    fn new(pid: rb_sys::pid_t, status: i32, error: i32) -> Result<Self, Error> {
        #[cfg(ruby_lt_3_0)]
        compile_error!("Process::Status is only compatible in Ruby 3.0+");

        let allocated = Self::allocate()?;
        let typed_data = magnus::RTypedData::from_value(allocated)
            .ok_or_else(|| new_base_error!("Failed to allocate process status"))
            .unwrap();
        let raw_typed_data = typed_data.as_raw() as *mut rb_sys::RTypedData;
        let raw_typed_data = unsafe { &mut *raw_typed_data };
        raw_typed_data.data = Self::build_struct_data(pid, status, error)?;

        Ok(Self(typed_data))
    }

    fn allocate() -> Result<Value, Error> {
        let klass = *memoize!(RClass: {
          let process: RModule = object().const_get("Process").unwrap();
          let klass = process.const_get("Status").unwrap();
          register_mark_object(klass);
          klass
        });

        klass.new_instance(())
    }

    fn build_struct_data(
        pid: rb_sys::pid_t,
        status: i32,
        error: i32,
    ) -> Result<*mut c_void, Error> {
        #[repr(C)]
        struct RbProcessStatus {
            pid: rb_sys::pid_t,
            status: i32,
            error: i32,
        }

        let status = RbProcessStatus { pid, status, error };
        let malloc_size = std::mem::size_of::<RbProcessStatus>();
        let mut ptr = std::ptr::null_mut();

        // Make sure we use the Ruby allocator to allocate the memory for the struct
        protect(|| {
            ptr = unsafe { rb_sys::ruby_xmalloc(malloc_size as _) };
            QNIL.as_raw()
        })?;

        unsafe { std::ptr::write_volatile(ptr as *mut RbProcessStatus, status) };

        Ok(ptr)
    }
}

impl Future for PidWaiter {
    type Output = Result<Value, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let registered_interest = self.signal.poll_recv(cx).is_pending();

            match self.try_wait() {
                Ok(Some(status)) => {
                    debug!(?status, "process exited");

                    return Poll::Ready(Ok(status.into_value()));
                }
                Ok(None) => {
                    debug!("process not exited yet");
                }
                Err(e) => {
                    error!(?e, "polling for next signal failed");
                }
            };

            // If our attempt to poll for the next signal was not ready, then
            // we've arranged for our task to get notified and we can bail out.
            if registered_interest {
                return Poll::Pending;
            } else {
                // Otherwise, if the signal stream delivered a signal to us, we
                // won't get notified at the next signal, so we'll loop and try
                // again.
                continue;
            }
        }
    }
}

impl IntoValue for ProcessStatus {
    fn into_value_with(self, _handle: &magnus::Ruby) -> Value {
        self.0.into_value()
    }
}
