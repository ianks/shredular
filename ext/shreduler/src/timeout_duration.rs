use magnus::{rb_sys::AsRawValue, TryConvert, Value};
use std::time::Duration;

/// Represents the amount of time to wait before timing out (in seconds to match Ruby semantics).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimeoutDuration(tokio::time::Duration);

const FAR_FUTURE: Duration = Duration::from_secs(86400 * 365 * 30);

impl TimeoutDuration {
    pub fn into_std(self) -> std::time::Duration {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == Duration::from_secs(0)
    }
}

impl TryConvert for TimeoutDuration {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        let dur = if value.is_nil() {
            FAR_FUTURE
        } else if value.class().as_raw() == magnus::class::integer().as_raw() {
            let value: u64 = value.try_convert()?;
            std::time::Duration::from_secs(value)
        } else {
            let float = value.try_convert::<f64>()?;
            std::time::Duration::from_secs_f64(float)
        };

        Ok(Self(dur))
    }
}
