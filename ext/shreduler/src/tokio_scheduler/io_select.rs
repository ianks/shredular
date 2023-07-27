use super::prelude::*;

impl TokioScheduler {
    /// Checks whether the specified IO objects are ready for the specified events within the given timeout.
    #[tracing::instrument]
    pub fn io_select(
        &self,
        _readables: RArray,
        _writables: RArray,
        _exceptables: RArray,
        _timeout: Option<TimeoutDuration>,
    ) -> Result<(RArray, RArray, RArray), Error> {
        crate::rtodo!("io_select")
    }
}
