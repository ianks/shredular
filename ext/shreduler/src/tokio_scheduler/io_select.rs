use std::future::IntoFuture;

use super::prelude::*;
use crate::{intern::object::empty_array, new_base_error, nilable::Nilable, ruby_io::RubyIoSet};
use magnus::{scan_args::scan_args, QNIL};
use tokio::io::Interest;

type IoSelectArgs = (
    Option<Nilable<RArray>>,
    Option<Nilable<RArray>>,
    Option<Nilable<RArray>>,
    Option<TimeoutDuration>,
);

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

        let timeout = timeout.unwrap_or(TimeoutDuration::far_future()).into_std();
        let readable_set = RubyIoSet::new_with_interest(readables, Interest::READABLE)?;
        let writable_set = RubyIoSet::new_with_interest(writables, Interest::WRITABLE)?;
        let exceptable_set = RubyIoSet::new_with_interest(exceptable, Interest::READABLE)?;
        let timeout_future = tokio::time::sleep(timeout);

        debug!(
            ?readable_set,
            ?writable_set,
            ?exceptable_set,
            "Created IO sets"
        );
        let readable_future = self.runtime()?.spawn(readable_set.into_future());
        let writable_future = self.runtime()?.spawn(writable_set.into_future());
        let exceptable_future = self.runtime()?.spawn(exceptable_set.into_future());

        let future = async move {
            let selected_ios = tokio::select! {
                exceptable_future = exceptable_future => {
                    debug!(?exceptable_future, "Exceptable IOs selected");
                    let result_exceptables = exceptable_future.map_err(|e| new_base_error!("Could not wait for exceptable IOs: {}", e))?;
                    Ok(*RArray::from_slice(&[
                        empty_array(),
                        empty_array(),
                        result_exceptables?,
                    ]))
                }
                readable_future = readable_future => {
                    debug!(?readable_future, "Readable IOs selected");
                    let result_readables = readable_future.map_err(|e| new_base_error!("Could not wait for readable IOs: {}", e))?;
                    Ok(*RArray::from_slice(&[
                        result_readables?,
                        empty_array(),
                        empty_array(),
                    ]))
                }
                writable_future = writable_future => {
                    debug!(?writable_future, "Writable IOs selected");
                    let result_writables = writable_future.map_err(|e| new_base_error!("Could not wait for writable IOs: {}", e))?;
                    Ok(*RArray::from_slice(&[
                        empty_array(),
                        result_writables?,
                        empty_array(),
                    ]))
                }
                _timeout_future = timeout_future => {
                    debug!("Timeout reached");
                    Ok(*QNIL)
                }
            };

            debug!(?selected_ios, "IOs selected");
            selected_ios.map(|r| r.into_value())
        };

        self.spawn_and_transfer(future)
    }
}
