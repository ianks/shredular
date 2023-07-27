use super::prelude::*;

impl TokioScheduler {
    /// Called when the current thread exits. The scheduler is expected to
    /// implement this method in order to allow all waiting fibers to finalize
    /// their execution.
    ///
    /// The suggested pattern is to implement the main event loop in the close method.
    #[tracing::instrument]
    pub fn close(rb_self: Obj<Self>) -> Result<(), Error> {
        Ok(())
    }

    /// Runs the scheduler until all fibers are completed.
    #[tracing::instrument]
    pub fn run(&self) -> Result<(), Error> {
        self.runtime()?.block_on(async move {
            while let Some(task) = self.futures_to_run.try_borrow_mut()?.join_next().await {
                match task {
                    Ok(fiber) => {
                        match fiber.resume() {
                            Ok(value) => {
                                info!(?value, "Fiber completed successfully");
                            }
                            Err(error) => {
                                error!(?error, "Error resuming fiber");
                                return Err(error);
                            }
                        };
                    }
                    Err(error) => {
                        error!(?error, "Could not join future");
                    }
                };
            }

            Ok(())
        })?;

        Ok(())
    }
}
