use magnus::QNIL;

use super::prelude::*;

impl TokioScheduler {
    /// Puts the current fiber to sleep for the specified duration.
    #[tracing::instrument]
    pub fn kernel_sleep(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
    ) -> Result<(), Error> {
        let future = async move {
            let dur = duration.into_std();
            tokio::time::sleep(dur).await;
            Ok(*QNIL)
        };

        self.spawn_and_transfer(future)?;
        Ok(())
    }
}
