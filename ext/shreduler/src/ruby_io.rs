use crate::{
    intern::{self},
    new_base_error,
};
use magnus::{Error, IntoValue, Value};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use rustix::fd::RawFd;
use tokio::io::{
    unix::{AsyncFd, AsyncFdReadyGuard},
    Interest,
};
use tracing::debug;

#[derive(Debug)]
pub struct RubyIo {
    value: Value,
    async_fd: AsyncFd<RawFd>,
}

impl RubyIo {
    pub fn new_with_interest(value: Value, interest: Interest) -> Result<Self, Error> {
        let _io: Value = intern::class::io().funcall(intern::id::try_convert(), (value,))?;
        let fileno: RawFd = value.funcall(intern::id::fileno(), ())?;
        let async_fd = AsyncFd::with_interest(fileno, interest).map_err(|e| {
            new_base_error!("Could not create AsyncFd from RawFd {}: {}", fileno, e)
        })?;

        Ok(Self { value, async_fd })
    }

    #[allow(dead_code)]
    pub fn set_nonblocking(self) -> Result<Self, Error> {
        let fd = *self.async_fd.get_ref();
        let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| {
            new_base_error!("Could not get file descriptor flags for {}: {}", fd, e)
        })?;
        let new_flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;

        fcntl(fd, FcntlArg::F_SETFL(new_flags)).map_err(|e| {
            new_base_error!("Could not set file descriptor flags for {}: {}", fd, e)
        })?;

        Ok(self)
    }

    pub async fn ready(
        &self,
        interest: Interest,
    ) -> Result<(Value, AsyncFdReadyGuard<'_, RawFd>), Error> {
        let guard = {
            let span = tracing::span!(
                tracing::Level::DEBUG,
                "waiting_for_interest",
                ?self,
                ?interest
            );
            let _ = span.enter();
            self.async_fd.ready(interest).await.map_err(|e| {
                new_base_error!(
                    "Could not wait for interest {:?} on RawFd {}: {}",
                    interest,
                    *self.async_fd.get_ref(),
                    e
                )
            })
        }?;

        debug!("IO is ready for interest {:?}", interest);

        Ok((self.value, guard))
    }
}

impl IntoValue for RubyIo {
    fn into_value_with(self, _handle: &magnus::Ruby) -> Value {
        self.value
    }
}
