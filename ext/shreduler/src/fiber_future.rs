use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;
use magnus::{Error, Value};
use tracing::trace;

use crate::fiber::{Fiber, Suspended};

pub type RubyFuture = Pin<Box<dyn Future<Output = Result<Value, Error>>>>;
pub type ResumableFiberFuture = Pin<Box<dyn Future<Output = ResumableFiber>>>;

#[derive(Debug)]
#[must_use]
pub struct ResumableFiber {
    fiber: Fiber<Suspended>,
    value: Result<Value, Error>,
}

impl ResumableFiber {
    fn new(fiber: Fiber<Suspended>, value: Result<Value, Error>) -> Self {
        Self { fiber, value }
    }

    pub fn resume(self) -> Result<Value, Error> {
        match self.value {
            Ok(value) => self.fiber.transfer((value,)),
            err => err,
        }
    }
}

/// A future that resumes a fiber when it is ready.
#[repr(C)]
pub struct FiberFuture {
    future: RubyFuture,
    fiber: Fiber<Suspended>,
}

impl FiberFuture {
    /// Creates a new `FiberFuture` from a `Fiber` and a future.
    pub fn new(
        fiber: Fiber<Suspended>,
        future: impl Future<Output = Result<Value, Error>> + 'static,
    ) -> Self {
        Self {
            future: Box::pin(future),
            fiber,
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
                trace!(?result, "Ruby future ready");
                let resumable = ResumableFiber::new(*fiber, result);
                Poll::Ready(resumable)
            }
            Poll::Pending => {
                trace!("Ruby future not ready");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
}
