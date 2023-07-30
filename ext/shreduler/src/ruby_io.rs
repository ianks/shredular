use std::{future::IntoFuture, pin::Pin};

use crate::{
    intern::{self, object::empty_array},
    new_base_error,
    nilable::Nilable,
    timeout_duration::TimeoutDuration,
};
use futures::{stream::FuturesUnordered, Future, StreamExt};
use magnus::{Error, IntoValue, RArray, TryConvert, Value};
use rustix::fd::RawFd;
use tokio::io::{unix::AsyncFd, Interest};
use tracing::{debug, error};

#[derive(Debug, Clone, Copy)]
pub struct RubyIoSet(Option<RArray>, Interest);

impl RubyIoSet {
    #[tracing::instrument]
    pub fn new_with_interest(
        value: Option<Nilable<RArray>>,
        interest: Interest,
    ) -> Result<Self, Error> {
        match value {
            Some(Nilable::Value(value)) => Ok(Self(Some(value), interest)),
            _ => Ok(Self(None, interest)),
        }
    }
}
impl IntoFuture for RubyIoSet {
    type Output = Result<RArray, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'static>>;

    #[tracing::instrument]
    fn into_future(self) -> Self::IntoFuture {
        let interest = self.1;
        match self.0 {
            None => {
                debug!(?interest, "IO set was empty, sleeping forever");
                Box::pin(async move {
                    tokio::time::sleep(TimeoutDuration::far_future().into_std()).await;
                    Ok(empty_array())
                })
            }
            Some(array) if array.is_empty() => {
                debug!(?interest, "IO set was empty, sleeping forever");
                Box::pin(async move {
                    tokio::time::sleep(TimeoutDuration::far_future().into_std()).await;
                    Ok(empty_array())
                })
            }
            Some(array) => {
                let mut futures = FuturesUnordered::new();

                for io in unsafe { array.as_slice() } {
                    let io = RubyIo::new_unchecked(*io);

                    let future = async move {
                        let async_fd = io.into_async_fd_with_interest(self.1)?;
                        let _guard = async_fd
                            .ready(self.1)
                            .await
                            .map_err(|e| new_base_error!("Could not wait for interest: {}", e))?;

                        let io = io.into_value();
                        debug!(?io, "IO from set is ready");

                        Ok::<_, Error>(io)
                    };

                    futures.push(future);
                }

                Box::pin(async move {
                    // Placeholder for the final result.
                    let result = RArray::new();

                    // Poll the FuturesUnordered stream until all futures have completed.
                    if let Some(io) = futures.next().await {
                        debug!(
                            ?futures,
                            ?io,
                            "IO from set has completed, pushing to result"
                        );

                        match io {
                            Ok(io) => {
                                debug!(?io, "Pushing IO to result array");
                                // Push the ready IO to the result array.
                                result.push(io).unwrap_or_else(|e| {
                                    error!("Error pushing to RArray: {}", e);
                                });
                            }
                            Err(error) => {
                                error!(
                                    ?error,
                                    "Error joining on task while waiting for IO readiness"
                                );
                            }
                        }
                    }

                    // futures.abort_all();

                    debug!(?result, "All IOs ready");
                    Ok(result)
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RubyIo(Value);

impl RubyIo {
    fn new_unchecked(value: Value) -> Self {
        Self(value)
    }

    pub fn into_async_fd_with_interest(self, interest: Interest) -> Result<AsyncFd<RawFd>, Error> {
        let fileno: RawFd = self.0.funcall(intern::id::fileno(), ())?;

        let fd = AsyncFd::with_interest(fileno, interest)
            .map_err(|e| new_base_error!("Could not create AsyncFd from RawFd: {}", e))?;

        Ok(fd)
    }
}

impl TryConvert for RubyIo {
    fn try_convert(value: Value) -> Result<Self, Error> {
        intern::class::io().funcall(intern::id::try_convert(), (value,))
    }
}

impl IntoValue for RubyIo {
    fn into_value_with(self, _handle: &magnus::Ruby) -> Value {
        self.0
    }
}
