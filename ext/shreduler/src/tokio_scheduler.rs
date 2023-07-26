use futures::{Future, TryFutureExt};
use std::cell::UnsafeCell;
use std::collections::{HashMap, VecDeque};
use std::mem::transmute;
use std::pin::Pin;
use tokio::task::{JoinHandle, JoinSet};

use magnus::{
    exception::standard_error, gc, typed_data::DataTypeBuilder, DataTypeFunctions, Error,
    ExceptionClass, RClass, TypedData,
};
use magnus::{prelude::*, DataType, IntoValue, RArray, Value, QNIL};
use tokio::runtime::{EnterGuard, Runtime};
use tracing::{debug, error, trace};

use crate::fiber::{Fiber, Unknown};
use crate::fiber_future::{FiberFuture, ResumableFiber};
use crate::scheduler_interface::Scheduler;
use crate::timeout_duration::TimeoutDuration;
use crate::{define_scheduler_methods, fiber_future, impl_typed_data_for_scheduler};

#[derive(Debug)]
pub struct TokioScheduler {
    root_fiber: Fiber<Unknown>,
    futures_to_run: UnsafeCell<JoinSet<ResumableFiber>>,
    runtime: Option<Runtime>,
    enter_guard: Option<EnterGuard<'static>>,
}

impl DataTypeFunctions for TokioScheduler {
    fn mark(&self) {
        gc::mark(self.root_fiber);
        // let futures_to_run = unsafe { &*self.futures_to_run.get() };

        // for (fiber, _) in futures_to_run {
        //     gc::mark(*fiber);
        // }
    }
}

impl TokioScheduler {
    /// Implementation of the `Scheduler` interface for the Tokio runtime.
    ///
    /// Internally, this scheduler uses the `tokio::runtime::Runtime` execute and
    /// poll futures: yielding control to other fibers when a future is not ready,
    /// and resuming the fiber when the future is ready.
    /// Creates a new Tokio scheduler.
    pub fn new() -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            // .worker_threads(1)
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| Error::new(base_error(), format!("{e}")))?;

        let enter_guard = runtime.enter();
        // SAFETY: the lifetime of the guard is tied to the lifetime of this
        // scheduler and Ruby object, so we consider it static.
        let enter_guard = unsafe { transmute::<_, EnterGuard<'static>>(enter_guard) };

        Ok(Self {
            root_fiber: Fiber::current().as_unknown(),
            futures_to_run: Default::default(),
            runtime: Some(runtime),
            enter_guard: Some(enter_guard),
        })
    }
}

macro_rules! rtodo {
    ($method:literal) => {
        Err(Error::new(
            base_error(),
            concat!("not implemented yet: ", $method),
        ))
    };
}

impl Scheduler for TokioScheduler {
    fn run(&self) -> Result<(), Error> {
        debug!("running Tokio scheduler");

        self.runtime()?.block_on(async move {
            while let Some(task) = self.futures_to_run_mut().join_next().await {
                match task {
                    Ok(fiber) => {
                        match fiber.resume() {
                            Ok(_) => {}
                            Err(e) => {
                                error!(?e, "error resuming fiber");
                                return Err(e);
                            }
                        };
                    }
                    Err(e) => {
                        error!(?e, "could not join future");
                    }
                };
            }

            Ok(())
        })?;

        Ok(())
    }

    fn address_resolve(&self, hostname: magnus::RString) -> Result<Value, Error> {
        let future = async move {
            debug!("resolving address");
            // See https://github.com/socketry/async/issues/180 for more details.
            let hostname = unsafe { hostname.as_str() }?;
            let hostname = hostname.split('%').next().unwrap_or(hostname);
            let mut split = hostname.splitn(2, ':');

            let Some(host) = split.next() else {
                        return Ok(RArray::new().into_value());
                    };

            if let Some(_port) = split.next() {
                // Match the behavior of MRI, which returns an empty array if the port is given
                return Ok(RArray::new().into_value());
            }

            let host_lookup = tokio::net::lookup_host((host, 80)).await;
            let host_lookup = host_lookup.map_err(|e| Error::new(base_error(), format!("{e}")));
            let addresses = RArray::new();

            for address in host_lookup? {
                trace!(?address, "found address");
                addresses.push(address.ip().to_string())?;
            }

            debug!(?addresses, "returning addresses");
            Ok(*addresses)
        };

        let timeout = Some(TimeoutDuration::from_secs(5));
        let future = with_timeout(timeout, future);
        let fiber_future = FiberFuture::new(future);
        self.futures_to_run_mut().spawn(fiber_future);
        self.root_fiber.check_suspended()?.transfer(())
    }

    fn block(
        &self,
        blocker: Value,
        timeout: Option<crate::timeout_duration::TimeoutDuration>,
    ) -> Result<bool, Error> {
        rtodo!("block")
    }

    fn unblock(&self, blocker: Value, fiber_to_wake: Value) -> Result<(), Error> {
        rtodo!("unblock")
    }

    fn io_wait(
        &self,
        io: std::os::fd::RawFd,
        interests: crate::scheduler_interface::IoInterests,
        timeout: Option<crate::timeout_duration::TimeoutDuration>,
    ) -> Result<Value, Error> {
        rtodo!("io_wait")
    }

    fn kernel_sleep(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
    ) -> Result<Value, Error> {
        rtodo!("kernel_sleep")
    }

    fn close(rb_self: magnus::typed_data::Obj<Self>) -> Result<(), Error> {
        trace!("calling close");
        Ok(())
    }

    fn process_wait(&self, pid: u32, flags: i32) -> Result<Value, Error> {
        rtodo!("process_wait")
    }

    fn timeout_after(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
        exception_class: ExceptionClass,
        message: magnus::RString,
    ) -> Result<Value, Error> {
        rtodo!("timeout_after")
    }
}
async fn with_timeout(
    timeout: Option<TimeoutDuration>,
    f: impl Future<Output = Result<Value, Error>>,
) -> Result<Value, Error> {
    if let Some(timeout) = timeout {
        let dur = timeout.into_std();
        match tokio::time::timeout(dur, f).await {
            Ok(result) => result,
            Err(_) => Err(Error::new(base_error(), format!("Timeout after {:?}", dur))),
        }
    } else {
        f.await
    }
}

impl TokioScheduler {
    fn runtime(&self) -> Result<&Runtime, Error> {
        self.runtime
            .as_ref()
            .ok_or_else(|| Error::new(base_error(), "scheduler is closed".to_owned()))
    }

    #[allow(clippy::mut_from_ref)]
    fn futures_to_run_mut(&self) -> &mut JoinSet<ResumableFiber> {
        unsafe { &mut *self.futures_to_run.get() }
    }
}

pub fn base_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("Error", standard_error()).unwrap();
        gc::register_mark_object(*c);
        c
    })
}

pub fn init() -> Result<(), Error> {
    tracing_subscriber::fmt::init();
    impl_typed_data_for_scheduler!(TokioScheduler);
    define_scheduler_methods!(TokioScheduler);

    Ok(())
}
