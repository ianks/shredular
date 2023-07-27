use magnus::{block::Proc, scan_args};

use super::prelude::*;

impl TokioScheduler {
    /// Schedules the given Proc to run in a separate non-blocking fiber.
    #[tracing::instrument]
    pub fn fiber(&self, args: &[Value]) -> Result<Fiber<Suspended>, Error> {
        let args = scan_args::scan_args::<(), (), (), (), (), Proc>(args)?;
        let block: Proc = args.block;
        Fiber::<Suspended>::spawn_nonblocking(block)
    }
}
