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
///   ready (e.g. I/O ready or sleep time elapsed).mod address_resolve;
mod address_resolve;
mod block_unblock;
mod close;
mod fiber_spawn;
mod io_read_write;
mod io_select;
mod io_wait;
mod kernel_sleep;
mod prelude;
mod process_wait;
mod timeout_after;

use futures::Future;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::fmt::Formatter;
use tracing::{debug, span, Level};

use tokio::sync::oneshot::Sender;
use tokio::task::{AbortHandle, JoinSet};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};
use tracing_tree::HierarchicalLayer;

use magnus::{gc, typed_data::DataTypeBuilder, DataTypeFunctions, Error, RClass, TypedData};
use magnus::{prelude::*, DataType, IntoValue, Value};
use tokio::runtime::Runtime;

use crate::errors::base_error;
use crate::fiber::{Fiber, Suspended, Unknown};
use crate::fiber_future::{FiberFuture, ResumableFiber};
use crate::gc_cell::GcCell;
use crate::new_base_error;

pub struct TokioScheduler {
    root_fiber: Fiber<Unknown>,
    futures_to_run: GcCell<JoinSet<ResumableFiber>>,
    runtime: UnsafeCell<Option<Runtime>>,
    pub blockers: GcCell<HashMap<Fiber<Suspended>, Sender<Value>>>,
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
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| new_base_error!("Could not build tokio runtime: {e}"))?;

        // TODO: should remove this since the runtime is not static
        let enter_guard = runtime.enter();
        std::mem::forget(enter_guard);

        Ok(Self {
            root_fiber: Fiber::current().as_unknown(),
            futures_to_run: Default::default(),
            runtime: UnsafeCell::new(Some(runtime)),
            blockers: Default::default(),
        })
    }
}

impl std::fmt::Debug for TokioScheduler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioScheduler")
            .field("root_fiber", &self.root_fiber.into_value())
            .field("futures_to_run", &self.futures_to_run)
            .field("blockers", &self.blockers)
            .finish_non_exhaustive()
    }
}

#[macro_export]
macro_rules! rtodo {
    ($method:literal) => {
        Err(magnus::Error::new(
            magnus::exception::not_imp_error(),
            concat!("TokioScheduler#", $method),
        ))
    };
}

impl TokioScheduler {
    fn runtime(&self) -> Result<&Runtime, Error> {
        if let Some(rt) = unsafe { &*self.runtime.get() } {
            Ok(rt)
        } else {
            Err(Error::new(base_error(), "runtime is closed"))
        }
    }

    #[tracing::instrument(skip(future))]
    pub fn spawn_and_transfer(
        &self,
        future: impl Future<Output = Result<Value, Error>> + 'static,
    ) -> Result<Value, Error> {
        {
            self.spawn(future)?;
        }
        debug!(?self, "Yielding to other fibers");
        let my_span = span!(Level::TRACE, "spawn_and_transfer");
        let my_span = my_span.enter();
        let ret = self.root_fiber.check_suspended()?.transfer(());
        drop(my_span);
        debug!(?self, "Resumed fiber that yielded");
        ret
    }

    #[tracing::instrument(skip(future))]
    pub fn spawn(
        &self,
        future: impl Future<Output = Result<Value, Error>> + 'static,
    ) -> Result<AbortHandle, Error> {
        let fiber_future = FiberFuture::new(future);
        Ok(self.futures_to_run.try_borrow_mut()?.spawn(fiber_future))
    }
}

unsafe impl Send for TokioScheduler {}
unsafe impl Sync for TokioScheduler {}

unsafe impl TypedData for TokioScheduler {
    fn class() -> RClass {
        *memoize!(RClass:{
          let c = magnus::define_class(stringify!(TokioScheduler),Default::default()).unwrap();
          c.undef_alloc_func();
          c
        })
    }
    fn data_type() -> &'static DataType {
        memoize!(DataType:{
          let mut builder = DataTypeBuilder::<Self>::new(stringify!(TokioScheduler));
          builder.mark();
          builder.free_immediately();
          builder.build()
        })
    }
}

impl DataTypeFunctions for TokioScheduler {
    fn mark(&self) {
        gc::mark(self.root_fiber);

        for fiber in self.blockers.borrow_for_gc().keys() {
            gc::mark(*fiber);
        }
    }
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

    let c = TokioScheduler::class();
    c.define_singleton_method("new", function!(TokioScheduler::new, 0))?;
    c.define_method(
        "address_resolve",
        method!(TokioScheduler::address_resolve, 1),
    )?;
    c.define_method("io_wait", method!(TokioScheduler::io_wait, 3))?;
    c.define_method("io_select", method!(TokioScheduler::io_select, -1))?;
    c.define_method("kernel_sleep", method!(TokioScheduler::kernel_sleep, 1))?;
    c.define_method("close", method!(TokioScheduler::close, 0))?;
    c.define_method("fiber", method!(TokioScheduler::fiber, -1))?;
    c.define_method("timeout_after", method!(TokioScheduler::timeout_after, 3))?;
    c.define_method("process_wait", method!(TokioScheduler::process_wait, 2))?;
    c.define_method("block", method!(TokioScheduler::block, 2))?;
    c.define_method("unblock", method!(TokioScheduler::unblock, 2))?;
    c.define_method("io_read", method!(TokioScheduler::io_read, 4))?;
    c.define_method("io_write", method!(TokioScheduler::io_write, 4))?;
    c.define_method("run", method!(TokioScheduler::run, 0))?;

    Ok(())
}
