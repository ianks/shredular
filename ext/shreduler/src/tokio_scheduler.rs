use std::{
    cell::RefCell,
    cell::{Cell, UnsafeCell},
    collections::{BTreeMap, BTreeSet, HashSet},
    future::{self, pending, IntoFuture},
    mem::transmute,
    os::fd::RawFd,
    pin::Pin,
    task::{Context, Poll},
    thread::JoinHandle,
    time::Duration,
};

use futures::{stream::FuturesUnordered, task::noop_waker, Future, Stream};
use magnus::{
    block::Proc,
    define_class, define_error,
    exception::{runtime_error, standard_error},
    gc,
    typed_data::{DataTypeBuilder, Obj},
    DataTypeFunctions, Error, ExceptionClass, IntoValue, RArray, RClass, TypedData, Value, QNIL,
};
use magnus::{prelude::*, DataType};
use tokio::{io::unix::AsyncFd, runtime::Runtime, task::LocalSet, time::Instant};
use tracing::{debug, error, info, trace};

use crate::{
    fiber::{Fiber, Running, State, Suspended, Unchecked},
    scheduler_interface::{IoInterests, Scheduler},
    timeout_duration::TimeoutDuration,
};

type TaskList = FuturesUnordered<Pin<Box<dyn Future<Output = Result<Value, Error>>>>>;

#[derive(Debug)]
pub struct TokioScheduler {
    event_loop_fiber: Option<Fiber<Unchecked>>,
    main_fiber: Fiber<Unchecked>,
    runtime: Option<Runtime>,
    futures_unordered: UnsafeCell<TaskList>,
    awaiting_fibers: RefCell<HashSet<Fiber<Suspended>>>,
}

// TODO: remove this by implementing a proper `Send` trait for `TaskList`
unsafe impl Send for TokioScheduler {}

unsafe impl magnus::TypedData for TokioScheduler {
    fn class() -> RClass {
        *magnus::memoize!(RClass: {
          let c = define_class("Shredular", Default::default()).unwrap();
          c.undef_alloc_func();
          c
        })
    }

    fn data_type() -> &'static DataType {
        magnus::memoize!(DataType: {
          let mut builder = DataTypeBuilder::<Self>::new("Shredular");
          builder.mark();
          builder.free_immediately();
          builder.build()
        })
    }
}

impl DataTypeFunctions for TokioScheduler {
    fn mark(&self) {
        gc::mark(self.main_fiber);

        if let Some(event_loop_fiber) = self.event_loop_fiber {
            gc::mark(event_loop_fiber);
        }

        for fiber in self.awaiting_fibers.borrow().iter() {
            gc::mark(*fiber);
        }
    }
}

impl TokioScheduler {
    /// Implementation of the `Scheduler` interface for the Tokio runtime.
    ///
    /// Internally, this scheduler uses the `tokio::runtime::Runtime` execute and
    /// poll futures: yielding control to other fibers when a future is not ready,
    /// and resuming the fiber when the future is ready.
    /// Creates a new Tokio scheduler.
    pub fn new() -> Result<Obj<Self>, magnus::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| magnus::Error::new(base_error(), format!("{e}")))?;

        let scheduler = Self {
            event_loop_fiber: None,
            main_fiber: Fiber::current().as_unchecked(),
            runtime: Some(runtime),
            futures_unordered: Default::default(),
            awaiting_fibers: Default::default(),
        };

        let scheduler = Obj::wrap(scheduler);

        TokioScheduler::spawn_event_loop_fiber(scheduler)
    }

    pub fn run_once(&self) -> Result<(), magnus::Error> {
        let rt = self.runtime().expect("runtime to be initialized");
        let rt_guard = rt.enter();
        let mut tasks = self.futures_unordered_mut();
        tokio::pin!(tasks);
        let waker = noop_waker();
        let mut context = Context::from_waker(&waker);

        /// use poll_next to poll all the futures in the FuturesUnordered
        /// and then poll the event loop fiber
        match tasks.poll_next(&mut context) {
            Poll::Ready(Some(Ok(v))) => {
                trace!("task finished successfully");
                Ok(())
            }
            Poll::Ready(Some(Err(e))) => {
                trace!("task finished with error: {:?}", e);
                Ok(())
            }
            Poll::Ready(None) => {
                trace!("no more tasks to run");
                drop(rt_guard);
                self.main_fiber
                    .check_suspended()
                    .expect("main fiber should be suspended")
                    .transfer(());
                Ok(())
            }
            Poll::Pending => {
                trace!("task is still pending");
                Ok(())
            }
        }
    }
}

impl TokioScheduler {
    pub fn event_loop_fiber(&self) -> Result<Fiber<Suspended>, magnus::Error> {
        self.event_loop_fiber
            .expect("no root fiber")
            .check_suspended()
    }

    pub fn runtime(&self) -> Option<&Runtime> {
        self.runtime.as_ref()
    }

    #[allow(clippy::mut_from_ref)]
    fn futures_unordered_mut(&self) -> &mut TaskList {
        // Safety: exclusive access is ensured by the GVL
        unsafe { &mut *self.futures_unordered.get() }
    }

    /// A fiber that checks any pending IO events and resumes the associated fibers.
    pub fn spawn_event_loop_fiber(rb_self: Obj<Self>) -> Result<Obj<Self>, magnus::Error> {
        let current_root = rb_self.get().event_loop_fiber;

        match current_root {
            Some(event_loop_fiber) => Ok(rb_self),
            None => {
                let proc = Proc::from_fn(move |_args, _block| {
                    loop {
                        let s = rb_self.get();
                        if s.awaiting_fibers.borrow().is_empty() {
                            trace!("no fibers awaiting IO, breaking event loop");

                            s.main_fiber
                                .check_suspended()
                                .expect("main fiber should be suspended")
                                .transfer(());
                        } else {
                            trace!("fibers awaiting IO, running event loop");
                            s.run_once()?;
                        }
                    }

                    Ok(QNIL)
                });
                let event_loop_fiber = Fiber::<Suspended>::new(proc)?.as_unchecked();

                // # Safety
                // We know that the fiber is not currently running, so and we
                // are the only ones that can access it. So we do this unsafely
                // rather than using a RefCell.
                let ptr = &rb_self.get().event_loop_fiber;
                let ptr = ptr as *const Option<Fiber<Unchecked>> as *mut Option<Fiber<Unchecked>>;
                unsafe { *ptr = Some(event_loop_fiber) };

                Ok(rb_self)
            }
        }
    }

    /// Registers a task to be executed by the scheduler, making sure to
    /// register the associated fiber as well so it can be marked for the GC,
    /// and resumed when the task is finished.
    fn register_task<F>(&self, task: F) -> Result<Value, magnus::Error>
    where
        F: Future<Output = Result<Value, Error>> + 'static,
    {
        let current_task_count = self.awaiting_fibers.borrow().len();
        trace!(current_task_count = ?current_task_count, "registering task with scheduler");
        let task_fiber = unsafe { Fiber::current().as_suspended() };
        self.awaiting_fibers.borrow_mut().insert(task_fiber);

        // # Safety
        // In order to be able to use `self` inside the closure, we need to
        // convince Rust that the the scheduler will still be alive when the
        // closure is called. We do this by transmuting `self` to a `&'static
        // Self` (which is safe because `self` is a `&'static TokioScheduler`),
        // and then transmuting it back to a `&Self` inside the closure (which
        // is safe because the closure is only called while `self` is still
        // alive).
        let static_self = unsafe { transmute::<&Self, &'static Self>(self) };

        let boxed_task = Box::pin(async move {
            debug!("scheduling task future");
            tokio::pin!(task);
            let result = task.await;
            debug!(result = ?result, "task finished, resuming fiber");
            let fiber = static_self
                .awaiting_fibers
                .borrow_mut()
                .take(&task_fiber)
                .expect("fiber not found");

            match result {
                Ok(v) => fiber.transfer((v,)),
                Err(e) => match e {
                    Error::Jump(_) => unreachable!("jump error should not be returned"),
                    Error::Error(exception_class, message) => fiber.raise(exception_class, message),
                    Error::Exception(exception) => {
                        todo!("raise exception")
                    }
                },
            }
        });

        let mut tasks = self.futures_unordered_mut();
        tasks.push(boxed_task);
        // Yield to the event loop fiber so it can run the task
        let ret = self.event_loop_fiber()?.transfer(())?;
        Ok(ret)
    }

    fn shutdown(rb_self: Obj<Self>) {
        info!("shutting down scheduler");

        let mut mut_obj = unsafe { (rb_self.get() as *const Self as *mut Self) };
        let mut_obj = unsafe { &mut *mut_obj };

        if let Some(rt) = mut_obj.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_millis(100));
            mut_obj.runtime = None;
        }

        mut_obj.event_loop_fiber = None;

        if Fiber::current().as_unchecked() != mut_obj.main_fiber {
            trace!("transferring to main fiber");
            mut_obj
                .main_fiber
                .check_suspended()
                .expect("main fiber should be suspended")
                .transfer(());
        }
    }
    //     let mut rt = unsafe { &mut *self.runtime.get() };
    //     if let Some(rt_instance) = rt.take() {
    //         rt_instance.shutdown_timeout(std::time::Duration::from_millis(100));
    //     }
    //     let mut event_loop_fiber = unsafe { &mut *self.event_loop_fiber };
    //     event_loop_fiber.take();
    // }
}

impl Scheduler for TokioScheduler {
    fn address_resolve(&self, hostname: String) -> Result<Value, magnus::Error> {
        let future = async move {
            let mut addresses = Vec::new();

            let host_lookup = tokio::net::lookup_host(&hostname).await;
            let host_lookup =
                host_lookup.map_err(|e| magnus::Error::new(base_error(), format!("{e}")));

            for address in host_lookup? {
                addresses.push(address.to_string());
            }

            Ok(*RArray::from_vec(addresses))
        };

        self.register_task(future)
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
                magnus::Error::new(base_error(), format!("Failed to create AsyncFd: {e}"))
            })?;
            if interests.contains(IoInterests::Readable) {
                let _ = async_fd.readable().await.map_err(|e| {
                    magnus::Error::new(base_error(), format!("Failed to wait for readable: {e}"))
                })?;
            }

            if interests.contains(IoInterests::Writable) {
                let _ = async_fd.writable().await.map_err(|e| {
                    magnus::Error::new(base_error(), format!("Failed to wait for writable: {e}"))
                })?;
            }

            Ok(interests.into_value())
        };

        let future_with_timeout = async move {
            let timeout = timeout.unwrap_or(TimeoutDuration::forever());

            tokio::select! {
              result = base_future => {
                result
              }
              _ = timeout.into_future() => {
                Err(magnus::Error::new(
                  base_error(),
                  format!("Timeout after {:?}", timeout),
                ))
              }
            }
        };

        self.register_task(future_with_timeout)
    }

    fn fiber(&self, block: Proc) -> Result<Fiber<Unchecked>, magnus::Error> {
        let fiber = Fiber::<Suspended>::new_nonblocking(block)?;
        fiber.transfer(())?;
        Ok(fiber.as_unchecked())
    }

    fn kernel_sleep(&self, duration: TimeoutDuration) {
        let rt = self.runtime().expect("runtime to be initialized");
        let _rt_guard = rt.enter();
        let _ = self.register_task(duration.into_future());
    }
}

pub fn base_error() -> ExceptionClass {
    *magnus::memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("Error", standard_error()).unwrap();
        gc::register_mark_object(*c);
        c
    })
}

pub fn init() -> Result<(), magnus::Error> {
    tracing_subscriber::fmt::init(); // TODO: dont make this global
    let c = TokioScheduler::class();

    c.define_singleton_method("new", function!(TokioScheduler::new, 0))?;
    c.define_method(
        "address_resolve",
        method!(TokioScheduler::address_resolve, 1),
    );
    c.define_method("io_wait", method!(TokioScheduler::io_wait, 3))?;
    c.define_method("kernel_sleep", method!(TokioScheduler::kernel_sleep, 1))?;
    c.define_method("shutdown", method!(TokioScheduler::shutdown, 0))?;

    Ok(())
}
