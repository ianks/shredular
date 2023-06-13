use futures::{task::noop_waker, FutureExt};
use magnus::{exception::runtime_error, rb_sys::AsRawValue, TryConvert, Value, QNIL};
use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Instant;
use tracing::debug;

/// Represents the amount of time to wait before timing out (in seconds to match Ruby semantics).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeoutDuration(tokio::time::Instant);

const FAR_FUTURE: Duration = Duration::from_secs(86400 * 365 * 30);

impl TimeoutDuration {
    /// A timeout duration of many, many seconds.
    pub fn forever() -> Self {
        Self(Instant::now() + FAR_FUTURE)
    }
}

impl IntoFuture for TimeoutDuration {
    type IntoFuture = TimeoutDurationFuture;
    type Output = Result<Value, magnus::Error>;

    fn into_future(self) -> Self::IntoFuture {
        let duration = self.0 - Instant::now();
        debug!(?duration, "Creating timeout duration future");
        TimeoutDurationFuture(Box::pin(tokio::time::sleep(duration)))
    }
}

#[derive(Debug)]
pub struct TimeoutDurationFuture(Pin<Box<tokio::time::Sleep>>);

impl Future for TimeoutDurationFuture {
    type Output = Result<Value, magnus::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let self_ = self.get_mut();
        let fut = &mut self_.0;
        tokio::pin!(fut);

        match fut.poll(cx) {
            Poll::Ready(_) => Poll::Ready(Ok(*QNIL)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl std::fmt::Debug for TimeoutDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl TryConvert for TimeoutDuration {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        let now = Instant::now();

        let until_duration = if value.is_nil() {
            FAR_FUTURE
        } else if value.class().as_raw() == magnus::class::integer().as_raw() {
            let value: u64 = value.try_convert()?;
            std::time::Duration::from_secs(value)
        } else {
            let float = value.try_convert::<f64>()?;
            std::time::Duration::from_secs_f64(float)
        };

        Ok(Self(now + until_duration))
    }
}
