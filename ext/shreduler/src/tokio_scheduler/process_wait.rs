use crate::rtodo;

use super::prelude::*;

impl TokioScheduler {
    /// Waits for the specified process with the given flags.
    #[tracing::instrument]
    pub fn process_wait(&self, pid: u32, flags: i32) -> Result<Value, Error> {
        rtodo!("process_wait")
    }
}
