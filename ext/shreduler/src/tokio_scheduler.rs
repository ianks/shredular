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
    gc, scan_args,
    typed_data::{DataTypeBuilder, Obj},
    DataTypeFunctions, Error, ExceptionClass, Integer, IntoValue, RArray, RClass, RString,
    TypedData, Value, QNIL,
};
use magnus::{prelude::*, DataType};
use tokio::{
    io::unix::AsyncFd, net::unix::SocketAddr, runtime::Runtime, task::LocalSet, time::Instant,
};
use tracing::{debug, error, info, trace};

use crate::{
    fiber::{Fiber, Running, State, Suspended, Unchecked},
    scheduler_interface::{IoInterests, Scheduler},
    timeout_duration::TimeoutDuration,
};

type TaskList = FuturesUnordered<Pin<Box<dyn Future<Output = Result<Value, Error>>>>>;

#[derive(Debug)]
pub struct TokioScheduler {
    event_loop_fiber: Cell<Option<Fiber<Unchecked>>>,
    main_fiber: Fiber<Unchecked>,
    runtime: Option<Runtime>,
    futures_unordered: UnsafeCell<TaskList>,
    awaiting_fibers: RefCell<HashSet<Fiber<Suspended>>>,
}

// TODO: remove this by implementing a proper `Send` trait for `TaskList`
unsafe impl Send for TokioScheduler {}
unsafe impl Sync for TokioScheduler {}

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

        if let Some(event_loop_fiber) = self.event_loop_fiber.get() {
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
    pub fn new() -> Result<Self, magnus::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| magnus::Error::new(base_error(), format!("{e}")))?;

        Ok(Self {
            event_loop_fiber: Cell::new(None),
            main_fiber: Fiber::current().as_unchecked(),
            runtime: Some(runtime),
            futures_unordered: Default::default(),
            awaiting_fibers: Default::default(),
        })
    }

    /// Polls the event loop fiber once.
    ///
    /// # Safety
    /// Caller must ensure this method is only called from event loop fiber.
    unsafe fn run_once(&self) -> Result<(), magnus::Error> {
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
                unsafe { self.main_fiber.as_suspended() }.transfer(());
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
            .get()
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

    fn set_event_loop_fiber(&self, fiber: Fiber<Unchecked>) {
        self.event_loop_fiber.set(Some(fiber));
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

        self.futures_unordered_mut().push(boxed_task);
        self.event_loop_fiber()?.transfer(())
    }

    fn drop_runtime(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_millis(100));
        }
    }

    fn shutdown(rb_self: Obj<Self>) {
        debug!("shutting down scheduler");

        let mut mut_obj = unsafe { (rb_self.get() as *const Self as *mut Self) };
        let mut_obj = unsafe { &mut *mut_obj };

        mut_obj.drop_runtime();
        mut_obj.event_loop_fiber = Cell::new(None);

        if let Ok(main_fiber) = mut_obj.main_fiber.check_suspended() {
            trace!("transferring to main fiber");
            main_fiber.transfer(()).expect("main fiber cant be resumed");
        }
    }
}

impl Scheduler for TokioScheduler {
    fn close(rb_self: Obj<Self>) -> Result<(), magnus::Error> {
        let sched = rb_self.get();
        let current_root = sched.event_loop_fiber.get();

        match current_root {
            Some(event_loop_fiber) => Ok(()),
            None => {
                let proc = Proc::from_fn(move |_args, _block| {
                    // SAFETY: we know that the scheduler is still alive since
                    // we are executing this proc
                    let static_self = unsafe { transmute::<_, &'static Self>(rb_self.get()) };

                    loop {
                        unsafe { static_self.run_once()? };
                    }

                    Ok(())
                });

                let event_loop_fiber = Fiber::<Suspended>::new(proc)?.as_unchecked();
                sched.set_event_loop_fiber(event_loop_fiber);
                Ok(())
            }
        }
    }

    fn address_resolve(&self, hostname: RString) -> Result<Value, magnus::Error> {
        let future = async move {
            // See https://github.com/socketry/async/issues/180 for more details.
            let hostname = unsafe { hostname.as_str() }?;
            let hostname = hostname.split('%').next().unwrap_or(hostname);
            let mut split = hostname.splitn(2, ':');

            let Some(host) = split.next() else {
                return Ok(RArray::new().into_value());
            };

            if let Some(port) = split.next() {
                // Match the behavior of MRI, which returns an empty array if the port is given
                return Ok(RArray::new().into_value());
            }

            let host_lookup = tokio::net::lookup_host((host, 80)).await;
            let host_lookup =
                host_lookup.map_err(|e| magnus::Error::new(base_error(), format!("{e}")));
            let addresses = RArray::new();

            for address in host_lookup? {
                addresses.push(address.ip().to_string())?;
            }

            Ok(*addresses)
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

        self.register_task(with_timeout(timeout, base_future))
    }

    fn fiber(&self, args: &[Value]) -> Result<Value, magnus::Error> {
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

async fn with_timeout<T, F>(timeout: Option<TimeoutDuration>, f: F) -> Result<T, Error>
where
    T: IntoValue,
    F: Future<Output = Result<T, Error>> + 'static,
{
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
    c.define_method("close", method!(TokioScheduler::close, 0))?;
    c.define_method("fiber", method!(TokioScheduler::fiber, -1))?;

    Ok(())
}
