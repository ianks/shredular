use magnus::{block::Proc, scan_args, QNIL};
use tracing::warn;

use super::prelude::*;

impl TokioScheduler {
    /// Schedules the given Proc to run in a separate non-blocking fiber.
    pub fn fiber(&self, vargs: &[Value]) -> Result<Value, Error> {
        let args = scan_args::scan_args::<(), (), (), (), (), Proc>(vargs)?;
        self.fiber_impl(args.block)
    }

    #[tracing::instrument]
    pub fn fiber_impl(&self, block: magnus::block::Proc) -> Result<Value, Error> {
        let transfer_back_to = Fiber::current().as_unknown();
        let wrapped_block = Proc::from_fn(move |args, _| {
            let result: Result<Value, Error> = block.call(args);
            match result {
                Ok(v) => transfer_back_to.check_suspended()?.transfer((v,)),
                Err(error) => {
                    let fiber = transfer_back_to.check_suspended()?;
                    fiber.raise(error)?;
                    Ok(*QNIL)
                }
            }
        });

        let fiber = Fiber::<Suspended>::new_nonblocking(wrapped_block)?;
        fiber.transfer(())?;
        warn!(?block, "fiber returned");
        Ok(fiber.into_value())
    }
}
