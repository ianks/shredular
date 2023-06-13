#![allow(unused)] // TODO: Remove this line

mod fiber;
mod scheduler_interface;
mod timeout_duration;
mod tokio_scheduler;

use magnus::{define_module, function, prelude::*, Error};

#[macro_use]
extern crate magnus;

fn hello(subject: String) -> String {
    format!("Hello from Rust, {}!", subject)
}

#[magnus::init]
fn init() -> Result<(), Error> {
    tokio_scheduler::init()?;
    // let module = define_module("Shreduler")?;
    // module.define_singleton_method("hello", function!(hello, 1))?;
    Ok(())
}
