use magnus::{exception::runtime_error, rb_sys::AsRawValue, TryConvert, Value};
use std::{
    future::{Future, IntoFuture},
    pin::Pin,
};

/// Represents the amount of time to wait before timing out (in seconds to match Ruby semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeoutDuration(std::time::Duration);

unsafe impl Send for TimeoutDuration {}

impl TimeoutDuration {
    /// The amount of time to wait before timing out.
    pub fn duration(self) -> std::time::Duration {
        self.0
    }
}

impl Default for TimeoutDuration {
    fn default() -> Self {
        Self(std::time::Duration::from_secs(60))
    }
}

impl IntoFuture for TimeoutDuration {
    type Output = Result<Value, magnus::Error>;

    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            tokio::time::sleep(self.duration()).await;

            Err(magnus::Error::new(
                runtime_error(),
                format!("Timeout after {}", self),
            ))
        })
    }
}

impl std::fmt::Display for TimeoutDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}s", self.0.as_secs_f64())
    }
}

impl TryConvert for TimeoutDuration {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        if value.class().as_raw() == magnus::class::integer().as_raw() {
            let value: u64 = value.try_convert()?;
            Ok(Self(std::time::Duration::from_secs(value)))
        } else {
            let float = value.try_convert::<f64>()?;
            Ok(Self(std::time::Duration::from_secs_f64(float)))
        }
    }
}
