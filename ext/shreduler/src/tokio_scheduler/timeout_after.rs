

use futures::Future;
use magnus::{
    block::{block_given, Proc},
    Class, ExceptionClass, QNIL,
};

use crate::new_timeout_error;

use super::prelude::*;

impl TokioScheduler {
    /// Executes a given block within the specified duration, raising an
    /// exception if the block's execution time exceeds the duration.
    #[tracing::instrument]
    pub fn timeout_after(
        &self,
        duration: crate::timeout_duration::TimeoutDuration,
        exception_class: ExceptionClass,
        message: magnus::RString,
    ) -> Result<Value, Error> {
        let fiber_to_timeout = Fiber::current().as_unknown();

        let timeout_future = async move {
            tokio::time::sleep(duration.into_std()).await;
            let error = exception_class.new_instance((message,))?;
            debug!(?error, ?duration, "Timeout waiting for block");
            let exception = exception_class.new_instance((message,))?;
            fiber_to_timeout
                .check_suspended()?
                .raise(Error::Exception(exception))
        };

        self.spawn(timeout_future)?;

        if block_given() {
            let blk: Proc = magnus::block::block_proc()?;
            blk.call(())
        } else {
            Ok(*QNIL)
        }
    }

    pub async fn with_timeout_no_raise(
        timeout: Option<TimeoutDuration>,
        f: impl Future<Output = Result<Value, Error>>,
    ) -> Result<Value, Error> {
        if let Some(timeout) = timeout {
            let dur = timeout.into_std();
            match tokio::time::timeout(dur, f).await {
                Ok(result) => result,
                Err(_) => Ok(*QNIL),
            }
        } else {
            f.await
        }
    }

    pub async fn with_timeout(
        desc: &'static str,
        timeout: Option<TimeoutDuration>,
        f: impl Future<Output = Result<Value, Error>>,
    ) -> Result<Value, Error> {
        if let Some(timeout) = timeout {
            let duration = timeout.into_std();
            match tokio::time::timeout(duration, f).await {
                Ok(result) => result,
                Err(error) => {
                    error!(?error, ?duration, "Timeout waiting for future");
                    Err(new_timeout_error!(
                        "timed out after {duration:?} seconds when {desc}",
                    ))
                }
            }
        } else {
            f.await
        }
    }
}
