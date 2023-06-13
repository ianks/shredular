use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Future;
use magnus::{Error, Value};
use tracing::trace;

use crate::fiber::{Fiber, Suspended};

pub type RubyFuture = Pin<Box<dyn Future<Output = Result<Value, Error>>>>;

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
    type Output = Result<Value, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let fut = &mut self.future;
        tokio::pin!(fut);

        match fut.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => match result {
                Ok(value) => {
                    trace!(result = ?value, "future ready to resume in fiber");
                    Poll::Ready(self.fiber.transfer((value,)))
                }
                Err(e) => Poll::Ready(self.fiber.raise(e)),
            },
        }
    }
}
