use std::future::ready;

use super::prelude::*;

impl TokioScheduler {
    /// Puts the current fiber to sleep for the specified duration.
    #[tracing::instrument]
    pub fn kernel_sleep(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
    ) -> Result<Value, Error> {
        let instant = tokio::time::Instant::now() + duration.into_std();

        if duration.is_zero() {
            let future = ready(Ok(0.into_value()));
            return self.spawn_and_transfer(future);
        }

        let future = async move {
            tokio::time::sleep_until(instant).await;
            let dur = duration.into_std();
            Ok(dur.as_secs().into_value())
        };

        self.spawn_and_transfer(future)
    }
}
