use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;
use magnus::{Error, Value};
use tracing::trace;

use crate::fiber::{Fiber, Unknown};

pub type RubyFuture = Pin<Box<dyn Future<Output = Result<Value, Error>>>>;

// TODO: look into using LocalSet
unsafe impl Send for FiberFuture {}
unsafe impl Sync for FiberFuture {}

#[derive(Debug)]
#[must_use]
pub struct ResumableFiber {
    fiber: Fiber<Unknown>,
    value: Result<Value, Error>,
}

impl ResumableFiber {
    fn new(fiber: Fiber<Unknown>, value: Result<Value, Error>) -> Self {
        Self { fiber, value }
    }

    pub fn resume(self) -> Result<Value, Error> {
        match self.value {
            Ok(value) => self.fiber.check_suspended()?.transfer((value,)),
            Err(err) => {
                let fiber = self.fiber.check_suspended()?;
                let ret = fiber.raise(err)?;
                Ok(ret)
            }
        }
    }
}

/// A future that resumes a fiber when it is ready.
#[repr(C)]
pub struct FiberFuture {
    future: RubyFuture,
    fiber: Fiber<Unknown>,
}

impl FiberFuture {
    /// Creates a new `FiberFuture` from a `Fiber` and a future.
    pub fn new(future: impl Future<Output = Result<Value, Error>> + 'static) -> Self {
        Self {
            future: Box::pin(future),
            fiber: Fiber::current().as_unknown(),
        }
    }
}

impl Future for FiberFuture {
    type Output = ResumableFiber;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Unsafe block is needed because the FiberFuture object is pinned, and
        // we need to move `future` and `fiber` out.  We're guaranteeing here
        // that we won't move out the value unless it's ready to produce an
        // output.
        let FiberFuture { future, fiber } = unsafe { self.get_unchecked_mut() };
        let future = unsafe { Pin::new_unchecked(future) };

        match Future::poll(future, cx) {
            Poll::Ready(result) => {
                trace!(?result, "FiberFuture ready");
                let resumable = ResumableFiber::new(*fiber, result);
                Poll::Ready(resumable)
            }
            Poll::Pending => {
                trace!("FiberFuture not ready");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}
