use futures::Future;
use magnus::{ExceptionClass, QNIL};

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
        crate::rtodo!("timeout_after")
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
        timeout: Option<TimeoutDuration>,
        f: impl Future<Output = Result<Value, Error>>,
    ) -> Result<Value, Error> {
        if let Some(timeout) = timeout {
            let dur = timeout.into_std();
            match tokio::time::timeout(dur, f).await {
                Ok(result) => result,
                Err(_) => Err(Error::new(base_error(), format!("timeout after {:?}", dur))),
            }
        } else {
            f.await
        }
    }
}
