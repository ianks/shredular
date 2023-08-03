use magnus::{block::Proc, scan_args};
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

        let fiber_block = if self.is_current_fiber_root() {
            block
        } else {
            // If we're already in a fiber, we need to wrap the block so we can
            // transfer back to the correct fiber
            Proc::from_fn(move |args, _| {
                let result: Result<Value, Error> = block.call(args);
                let transfer_back_to = transfer_back_to.check_suspended()?;

                match result {
                    Ok(v) => transfer_back_to.transfer((v,)),
                    Err(error) => transfer_back_to.raise(error),
                }
            })
        };

        let fiber = Fiber::<Suspended>::new_nonblocking(fiber_block)?;
        fiber.transfer(())?;
        warn!(?block, "fiber returned");
        Ok(fiber.into_value())
    }

    /// Returns true if the current fiber is the root fiber.
    pub fn is_current_fiber_root(&self) -> bool {
        self.root_fiber == Fiber::current().as_unknown()
    }
}
