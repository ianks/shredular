use tokio::sync::oneshot;

use super::prelude::*;

impl TokioScheduler {
    /// Invoked by methods like Thread.join, and by Mutex, to signify that
    /// current Fiber is blocked until further notice (e.g. unblock) or until
    /// timeout has elapsed.
    ///
    /// - `blocker` is what we are waiting on, informational only (for debugging and
    ///   logging). There are no guarantee about its value.
    ///
    /// - Expected to return boolean, specifying whether the blocking operation was successful or not.
    #[tracing::instrument]
    pub fn block(
        rb_self: Obj<Self>,
        _blocker: Value,
        timeout: Option<crate::timeout_duration::TimeoutDuration>,
    ) -> Result<Value, Error> {
        let rx = {
            let (tx, rx) = oneshot::channel();
            let fiber = unsafe { Fiber::current().as_suspended() };
            rb_self.get().blockers.try_borrow_mut()?.insert(fiber, tx);
            rx
        };

        let future = async move {
            let scheduler = rb_self.get();
            let result = Self::with_timeout(timeout, async move {
                let result = rx
                    .await
                    .map_err(|_| Error::new(base_error(), "could not unblock fiber"))?;
                Ok(result)
            })
            .await;

            let fiber = unsafe { Fiber::current().as_suspended() };
            scheduler.blockers.try_borrow_mut()?.remove(&fiber);
            result
        };

        rb_self.get().spawn_and_transfer(future)
    }

    /// Invoked to wake up Fiber previously blocked with block (for example,
    /// Mutex#lock calls block and Mutex#unlock calls unblock). The scheduler should
    /// use the fiber parameter to understand which fiber is unblocked.
    ///
    /// blocker is what was awaited for, but it is informational only (for debugging
    /// and logging), and it is not guaranteed to be the same value as the blocker
    /// for block
    #[tracing::instrument]
    pub fn unblock(&self, blocker: Value, fiber_to_wake: Value) -> Result<(), Error> {
        let fiber = Fiber::<Suspended>::from_value(fiber_to_wake)?.check_suspended()?;

        self.blockers
            .try_borrow_mut()?
            .remove(&fiber)
            .and_then(|tx| {
                tx.send(true.into())
                    .map_err(|_| Error::new(base_error(), "could not unblock fiber"))
                    .ok()
            });

        Ok(())
    }
}
