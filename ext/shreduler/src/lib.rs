mod fiber;
mod fiber_future;
mod gc_cell;
mod scheduler_interface;
mod timeout_duration;
mod tokio_scheduler;

use magnus::Error;

#[macro_use]
extern crate magnus;

#[magnus::init]
fn init() -> Result<(), Error> {
    tokio_scheduler::init()?;
    // let module = define_module("Shreduler")?;
    // module.define_singleton_method("hello", function!(hello, 1))?;
    Ok(())
}
