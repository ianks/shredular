use super::prelude::*;
use crate::{intern::object::empty_array, new_base_error, nilable::Nilable, ruby_io::RubyIo};
use magnus::{scan_args::scan_args, QNIL};
use tokio::{io::Interest, task::JoinSet};

type IoSelectArgs = (
    Option<Nilable<RArray>>,
    Option<Nilable<RArray>>,
    Option<Nilable<RArray>>,
    Option<TimeoutDuration>,
);

pub struct RubyIoSet {
    futures: JoinSet<Result<(Value, Interest), Error>>,
}

impl RubyIoSet {
    pub fn new() -> Self {
        Self {
            futures: JoinSet::new(),
        }
    }

    pub fn push_all(&mut self, array: RArray, interest: Interest) {
        for io in array.each() {
            let future = async move {
                let io = RubyIo::new_with_interest(io?, interest)?;
                let (ruby_io, _guard) = io.ready(interest).await?;
                Ok::<_, Error>((ruby_io, interest))
            };

            self.futures.spawn(future);
        }
    }

    pub fn len(&self) -> usize {
        self.futures.len()
    }
}

impl TokioScheduler {
    /// Non-blocking version of Ruby's `IO.select`, using tokio.
    ///
    /// Invoked by IO.select to ask whether the specified descriptors are ready
    /// for specified events within the specified timeout.  Expected to return
    /// the 3-tuple of Array of IOs that are ready.
    #[tracing::instrument]
    pub fn io_select(&self, args: &[Value]) -> Result<Value, Error> {
        let args = scan_args::<(), IoSelectArgs, (), (), (), ()>(args)?;
        let (readables, writables, exceptable, timeout) = args.optional;
        let mut io_set = RubyIoSet::new();

        if let Some(Nilable::Value(readables_array)) = readables {
            debug!(?readables_array, "Selecting readable IOs");
            io_set.push_all(readables_array, Interest::READABLE);
        }

        if let Some(Nilable::Value(writables_array)) = writables {
            debug!(?writables_array, "Selecting writable IOs");
            io_set.push_all(writables_array, Interest::WRITABLE);
        }

        if let Some(Nilable::Value(exceptables_array)) = exceptable {
            debug!(?exceptables_array, "Selecting exceptable IOs");
            io_set.push_all(exceptables_array, Interest::READABLE | Interest::WRITABLE);
        }

        debug!("Created Ruby IO set with {} IOs", io_set.len());

        let future = async move {
            debug!("Waiting for any IOs to be ready");

            let result = match io_set.futures.join_next().await {
                None => {
                    debug!("No IOs are ready");
                    Ok(*QNIL)
                }
                Some(Err(e)) => {
                    error!(?e, "Error joining futures");
                    Err(new_base_error!("Error joining futures: {}", e))
                }
                Some(Ok(Err(e))) => {
                    error!(?e, "Error waiting for IO to be ready");
                    Err(new_base_error!("Error waiting for IO to be ready: {}", e))
                }
                Some(Ok(Ok((ruby_io, interest)))) => {
                    debug!(?ruby_io, ?interest, "IO is ready");

                    if interest == Interest::READABLE | Interest::WRITABLE {
                        debug!("IO is ready for both reading and writing");
                        Ok(RArray::from_slice(&[
                            empty_array(),
                            empty_array(),
                            RArray::from_slice(&[ruby_io]),
                        ])
                        .into_value())
                    } else if interest == Interest::READABLE {
                        Ok(RArray::from_slice(&[
                            RArray::from_slice(&[ruby_io]),
                            empty_array(),
                            empty_array(),
                        ])
                        .into_value())
                    } else if interest == Interest::WRITABLE {
                        Ok(RArray::from_slice(&[
                            empty_array(),
                            RArray::from_slice(&[ruby_io]),
                            empty_array(),
                        ])
                        .into_value())
                    } else {
                        unreachable!("Unexpected interest");
                    }
                }
            };

            io_set.futures.abort_all();

            result
        };

        let future = Self::with_timeout_no_raise(timeout, future);

        self.spawn_and_transfer(future)
    }
}
