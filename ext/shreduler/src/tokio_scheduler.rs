use futures::Future;
use magnus::typed_data::Obj;
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::fmt::Formatter;
use std::mem::transmute;

use tokio::sync::oneshot::{self, Sender};
use tokio::task::JoinSet;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};
use tracing_tree::HierarchicalLayer;

use magnus::{
    exception::standard_error, gc, typed_data::DataTypeBuilder, DataTypeFunctions, Error,
    ExceptionClass, RClass, TypedData,
};
use magnus::{prelude::*, DataType, IntoValue, RArray, Value, QNIL};
use tokio::runtime::{EnterGuard, Runtime};
use tracing::{debug, error, info, trace};

use crate::fiber::{Fiber, Suspended, Unknown};
use crate::fiber_future::{FiberFuture, ResumableFiber};
use crate::scheduler_interface::Scheduler;
use crate::timeout_duration::TimeoutDuration;
use crate::{define_scheduler_methods, impl_typed_data_for_scheduler};

pub struct TokioScheduler {
    root_fiber: Fiber<Unknown>,
    futures_to_run: UnsafeCell<JoinSet<ResumableFiber>>,
    runtime: Option<Runtime>,
    blockers: RefCell<HashMap<Fiber<Suspended>, Sender<Value>>>,
    _enter_guard: Option<EnterGuard<'static>>, // needed for time::sleep
}

impl std::fmt::Debug for TokioScheduler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioScheduler")
            .field("root_fiber", &self.root_fiber)
            .field("futures_to_run", &self.futures_to_run_mut())
            .field("runtime", &"...")
            .field("_enter_guard", &"...")
            .field("blockers", &self.blockers)
            .finish()
    }
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

        // SAFETY: the lifetime of the guard is tied to the lifetime of this
        // scheduler and Ruby object, so we consider it static.
        let enter_guard = runtime.enter();
        let enter_guard = unsafe { transmute::<_, EnterGuard<'static>>(enter_guard) };

        Ok(Self {
            root_fiber: Fiber::current().as_unknown(),
            futures_to_run: Default::default(),
            runtime: Some(runtime),
            blockers: Default::default(),
            _enter_guard: Some(enter_guard),
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
    #[tracing::instrument]
    fn run(&self) -> Result<(), Error> {
        self.runtime()?.block_on(async move {
            while let Some(task) = self.futures_to_run_mut().join_next().await {
                match task {
                    Ok(fiber) => {
                        match fiber.resume() {
                            Ok(value) => {
                                info!(?value, "Fiber completed successfully");
                            }
                            Err(error) => {
                                error!(?error, "Error resuming fiber");
                                return Err(error);
                            }
                        };
                    }
                    Err(error) => {
                        error!(?error, "Could not join future");
                    }
                };
            }

            Ok(())
        })?;

        Ok(())
    }

    #[tracing::instrument]
    fn address_resolve(&self, hostname: magnus::RString) -> Result<Value, Error> {
        let future = async move {
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

        self.spawn_and_transfer(future)
    }

    /// Invoked by methods like Thread.join, and by Mutex, to signify that
    /// current Fiber is blocked until further notice (e.g. unblock) or until
    /// timeout has elapsed.
    ///
    /// - `blocker` is what we are waiting on, informational only (for debugging and
    ///   logging). There are no guarantee about its value.
    ///
    /// - Expected to return boolean, specifying whether the blocking operation was successful or not.   #[tracing::instrument]
    fn block(
        rb_self: Obj<Self>,
        _blocker: Value,
        timeout: Option<crate::timeout_duration::TimeoutDuration>,
    ) -> Result<Value, Error> {
        let (tx, rx) = oneshot::channel();
        let fiber = unsafe { Fiber::current().as_suspended() };
        let scheduler = rb_self.get();

        scheduler.blockers.borrow_mut().insert(fiber, tx);

        let future = async move {
            let scheduler = rb_self.get();
            let result = Self::with_timeout(timeout, async move {
                let result = rx
                    .await
                    .map_err(|_| Error::new(base_error(), "could not unblock fiber"))?;
                Ok(result)
            })
            .await;

            let fiber = unsafe { Fiber::current().as_suspended() };
            scheduler.blockers.borrow_mut().remove(&fiber);
            result
        };

        scheduler.spawn_and_transfer(future)
    }

    #[tracing::instrument]
    fn unblock(&self, blocker: Value, fiber_to_wake: Value) -> Result<(), Error> {
        if let Ok(fiber) = Fiber::<Suspended>::from_value(fiber_to_wake) {
            let fiber = fiber.check_suspended()?;
            let mut blockers = self.blockers.borrow_mut();

            if let Some(tx) = blockers.remove(&fiber) {
                drop(blockers);
                tx.send(true.into())
                    .map_err(|_| Error::new(base_error(), "could not unblock fiber"))?;
            }
        }

        Ok(())
    }

    #[tracing::instrument]
    fn io_wait(
        &self,
        io: std::os::fd::RawFd,
        interests: crate::scheduler_interface::IoInterests,
        timeout: Option<crate::timeout_duration::TimeoutDuration>,
    ) -> Result<Value, Error> {
        rtodo!("io_wait")
    }

    #[tracing::instrument]
    fn kernel_sleep(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
    ) -> Result<(), Error> {
        let future = async move {
            let dur = duration.into_std();
            tokio::time::sleep(dur).await;
            Ok(*QNIL)
        };

        self.spawn_and_transfer(future)?;
        Ok(())
    }

    #[tracing::instrument]
    fn close(rb_self: magnus::typed_data::Obj<Self>) -> Result<(), Error> {
        trace!("calling close");
        Ok(())
    }

    #[tracing::instrument]
    fn process_wait(&self, pid: u32, flags: i32) -> Result<Value, Error> {
        rtodo!("process_wait")
    }

    #[tracing::instrument]
    fn timeout_after(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
        exception_class: ExceptionClass,
        message: magnus::RString,
    ) -> Result<Value, Error> {
        rtodo!("timeout_after")
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

    fn spawn_and_transfer(
        &self,
        future: impl Future<Output = Result<Value, Error>> + 'static,
    ) -> Result<Value, Error> {
        let fiber_future = FiberFuture::new(future);
        self.futures_to_run_mut().spawn(fiber_future);
        self.root_fiber.check_suspended()?.transfer(())
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
}

pub fn base_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("Error", standard_error()).unwrap();
        gc::register_mark_object(*c);
        c
    })
}

pub fn init() -> Result<(), Error> {
    Registry::default()
        .with(EnvFilter::from_default_env())
        .with(
            HierarchicalLayer::new(2)
                .with_targets(true)
                .with_bracketed_fields(true),
        )
        .init();
    impl_typed_data_for_scheduler!(TokioScheduler);
    define_scheduler_methods!(TokioScheduler);

    Ok(())
}
