mod errors;
mod fiber;
mod fiber_future;
mod gc_cell;
mod intern;
mod io_buffer;
mod nilable;
mod ruby_io;
mod thread;
mod timeout_duration;
mod tokio_scheduler;

use magnus::Error;

#[macro_use]
extern crate magnus;

#[magnus::init]
fn init() -> Result<(), Error> {
    tokio_scheduler::init()?;
    Ok(())
}
