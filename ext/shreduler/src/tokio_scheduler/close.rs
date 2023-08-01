use crate::thread::check_interrupts;

use super::prelude::*;
use std::time::Duration;
use tracing::warn;

impl TokioScheduler {
    /// Called when the current thread exits. The scheduler is expected to
    /// implement this method in order to allow all waiting fibers to finalize
    /// their execution.
    ///
    /// The suggested pattern is to implement the main event loop in the close method.
    #[tracing::instrument]
    pub fn close(rb_self: Obj<Self>) -> Result<(), Error> {
        let scheduler = rb_self.get();
        scheduler.run()?;
        let mut_runtime = unsafe { &mut *scheduler.runtime.get() };

        if let Some(rt) = mut_runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(5));
        }

        Ok(())
    }

    /// Runs the scheduler until all fibers are completed.
    #[tracing::instrument]
    pub fn run(&self) -> Result<(), Error> {
        self.runtime()?.block_on(async move {
            loop {
                let task = {
                    let mut futures_to_run = self.futures_to_run.try_borrow_mut()?;
                    tokio::select! {
                        task = futures_to_run.join_next() => task,
                        Err(error) = check_interrupts() => {
                            warn!(?error, "Thread was interrupted, aborting {} pending task(s)", futures_to_run.len());
                            futures_to_run.abort_all();
                            return Err(error)
                        }
                    }
                };

                match task {
                    Some(Ok(fiber)) => {
                        match fiber.resume_if_alive() {
                            Ok(Some(value)) => {
                                info!(?value, "Fiber completed successfully");
                            }
                            Ok(None) => {
                                warn!("Fiber is dead, could not resume");
                            }
                            Err(error) => {
                                error!(?error, "Error resuming fiber");
                                return Err(error);
                            }
                        };
                    }
                    Some(Err(error)) => {
                        error!(?error, "Could not join future");
                    }
                    _ => {
                        info!("No more fibers to run");
                        return Ok(());
                    }
                };
            }
        })
    }
}
