use std::{
    cell::{Cell, UnsafeCell},
    os::fd::RawFd,
    task::Context,
};

use futures::{stream::FuturesUnordered, task::noop_waker, Future};
use magnus::{
    block::Proc,
    define_class,
    exception::standard_error,
    gc, scan_args,
    typed_data::{DataTypeBuilder, Obj},
    DataTypeFunctions, Error, ExceptionClass, Integer, IntoValue, RArray, RClass, RString,
    TypedData, Value,
};
use magnus::{prelude::*, DataType};
use tokio::{io::unix::AsyncFd, runtime::Runtime};
use tracing::{debug, trace};

use crate::{
    fiber::{Fiber, Suspended, Unknown},
    fiber_future::{FiberFuture, RubyFuture},
    scheduler_interface::{IoInterests, Scheduler},
    timeout_duration::TimeoutDuration,
};

type TaskList = FuturesUnordered<RubyFuture>;

#[derive(Debug, Clone, Copy)]
enum SchedulerStatus {
    Running,
    Stopped,
}

#[derive(Debug)]
pub struct TokioScheduler {
    status: Cell<SchedulerStatus>,
    event_loop_fiber: Cell<Option<Fiber<Unknown>>>,
    main_fiber: Fiber<Unknown>,
    runtime: Option<Runtime>,
    futures_unordered: UnsafeCell<TaskList>,
}

// TODO: remove this by implementing a proper `Send` trait for `TaskList`
unsafe impl Send for TokioScheduler {}
unsafe impl Sync for TokioScheduler {}

unsafe impl TypedData for TokioScheduler {
    fn class() -> RClass {
        *memoize!(RClass: {
          let c = define_class("Shredular", Default::default()).unwrap();
          c.undef_alloc_func();
          c
        })
    }

    fn data_type() -> &'static DataType {
        memoize!(DataType: {
          let mut builder = DataTypeBuilder::<Self>::new("Shredular");
          builder.mark();
          builder.free_immediately();
          builder.build()
        })
    }
}

impl DataTypeFunctions for TokioScheduler {
    fn mark(&self) {
        if let Some(event_loop_fiber) = self.event_loop_fiber.get() {
            gc::mark(event_loop_fiber);
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
    pub fn new() -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| Error::new(base_error(), format!("{e}")))?;

        Ok(Self {
            status: Cell::new(SchedulerStatus::Stopped),
            event_loop_fiber: Cell::new(None),
            main_fiber: Fiber::current().as_unchecked(),
            runtime: Some(runtime),
            futures_unordered: Default::default(),
        })
    }

    /// Polls the event loop fiber once.
    ///
    /// # Safety
    /// Caller must ensure this method is only called from event loop fiber.
    unsafe fn run_once(&self) -> Result<(), Error> {
        let _rt = self.runtime().expect("runtime to be initialized").enter();
        let tasks = self.futures_unordered_mut();
        tokio::pin!(tasks);

        for task in tasks.iter_mut() {
            trace!("polling task");
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);
            tokio::pin!(task);
            let _ = task.poll(&mut context);
        }

        Ok(())
    }

    fn event_loop_fiber(&self) -> Result<Fiber<Suspended>, Error> {
        self.event_loop_fiber
            .get()
            .expect("no root fiber")
            .check_suspended()
    }

    fn runtime(&self) -> Option<&Runtime> {
        self.runtime.as_ref()
    }

    #[allow(clippy::mut_from_ref)]
    fn futures_unordered_mut(&self) -> &mut TaskList {
        // Safety: exclusive access is ensured by the GVL
        unsafe { &mut *self.futures_unordered.get() }
    }

    fn futures_unordered(&self) -> &TaskList {
        // Safety: access is ensured by the GVL
        unsafe { &*self.futures_unordered.get() }
    }

    fn set_event_loop_fiber(&self, fiber: Fiber<Unknown>) {
        self.event_loop_fiber.set(Some(fiber));
    }

    /// Registers a task to be executed by the scheduler, making sure to
    /// register the associated fiber as well so it can be marked for the GC,
    /// and resumed when the task is finished.
    fn register_task(
        &self,
        task: impl Future<Output = Result<Value, Error>> + 'static,
    ) -> Result<Value, Error> {
        {
            let current_task_count = self.futures_unordered().len();
            trace!(current_task_count = ?current_task_count, "registering task with scheduler");
            let task_fiber = unsafe { Fiber::current().as_suspended() };
            let fiber_future = FiberFuture::new(task_fiber, task);

            self.futures_unordered_mut().push(Box::pin(fiber_future));
        }

        self.event_loop_fiber()?.transfer(())
    }

    fn drop_runtime(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_secs(5));
        }
    }

    fn shutdown(rb_self: Obj<Self>) {
        debug!("shutting down scheduler");

        let mut_obj = rb_self.get() as *const Self as *mut Self;
        let mut_obj = unsafe { &mut *mut_obj };

        mut_obj.status.set(SchedulerStatus::Stopped);
        mut_obj.drop_runtime();
        mut_obj.event_loop_fiber = Cell::new(None);

        if let Ok(main_fiber) = mut_obj.main_fiber.check_suspended() {
            trace!("transferring to main fiber");
            main_fiber.transfer(()).expect("main fiber cant be resumed");
        }
    }
}

impl Scheduler for TokioScheduler {
    fn close(rb_self: Obj<Self>) -> Result<(), Error> {
        let sched = rb_self.get();
        let current_root = sched.event_loop_fiber.get();

        match current_root {
            Some(_event_loop_fiber) => Ok(()),
            None => {
                let proc = Proc::from_fn(move |_args, _block| {
                    let sched = rb_self.get();
                    sched.status.set(SchedulerStatus::Running);

                    while matches!(sched.status.get(), SchedulerStatus::Running) {
                        // SAFETY: we are in the event loop fiber so we can call run_once
                        unsafe { sched.run_once()? };
                    }

                    Ok(())
                });

                let event_loop_fiber = Fiber::<Suspended>::new(proc)?.as_unchecked();
                sched.set_event_loop_fiber(event_loop_fiber);
                Ok(())
            }
        }
    }

    fn address_resolve(&self, hostname: RString) -> Result<Value, Error> {
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
                addresses.push(address.ip().to_string())?;
            }

            Ok(*addresses)
        };

        self.register_task(Box::pin(future))
    }

    fn io_wait(
        &self,
        io: RawFd,
        interests: IoInterests,
        timeout: Option<TimeoutDuration>,
    ) -> Result<Value, Error> {
        let base_future = async move {
            let tokio_interests = tokio::io::Interest::from(interests);
            let async_fd = AsyncFd::with_interest(io, tokio_interests)
                .map_err(|e| Error::new(base_error(), format!("Failed to create AsyncFd: {e}")))?;
            if interests.contains(IoInterests::Readable) {
                let _ = async_fd.readable().await.map_err(|e| {
                    Error::new(base_error(), format!("Failed to wait for readable: {e}"))
                })?;
            }

            if interests.contains(IoInterests::Writable) {
                let _ = async_fd.writable().await.map_err(|e| {
                    Error::new(base_error(), format!("Failed to wait for writable: {e}"))
                })?;
            }

            Ok(interests.into_value())
        };

        self.register_task(with_timeout(timeout, base_future))
    }

    fn fiber(&self, args: &[Value]) -> Result<Value, Error> {
        let args = scan_args::scan_args::<(), (), (), (), (), Proc>(args)?;
        let block: Proc = args.block;
        let fiber = Fiber::<Suspended>::new_nonblocking(block)?;
        fiber.transfer(())
    }

    fn kernel_sleep(&self, duration: TimeoutDuration) -> Result<Value, Error> {
        let _rt = self.runtime().expect("runtime to be initialized").enter();

        self.register_task(async move {
            let dur = duration.into_std();
            tokio::time::sleep(dur).await;
            let ruby_int = Integer::from_u64(dur.as_secs());
            Ok(*ruby_int)
        })
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

pub fn base_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("Error", standard_error()).unwrap();
        gc::register_mark_object(*c);
        c
    })
}

pub fn init() -> Result<(), Error> {
    tracing_subscriber::fmt::init(); // TODO: dont make this global
    let c = TokioScheduler::class();

    c.define_singleton_method("new", function!(TokioScheduler::new, 0))?;
    c.define_method(
        "address_resolve",
        method!(TokioScheduler::address_resolve, 1),
    )?;
    c.define_method("io_wait", method!(TokioScheduler::io_wait, 3))?;
    c.define_method("kernel_sleep", method!(TokioScheduler::kernel_sleep, 1))?;
    c.define_method("shutdown", method!(TokioScheduler::shutdown, 0))?;
    c.define_method("close", method!(TokioScheduler::close, 0))?;
    c.define_method("fiber", method!(TokioScheduler::fiber, -1))?;

    Ok(())
}
