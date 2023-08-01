use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    intern::{self},
    new_base_error,
};
use magnus::{Error, IntoValue, Value};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use rustix::fd::{AsRawFd, FromRawFd, RawFd};
use tokio::io::{
    unix::{AsyncFd, AsyncFdReadyGuard},
    AsyncRead, AsyncWrite, Interest, ReadBuf,
};
use tracing::debug;

#[derive(Debug)]
pub enum BackingIo {
    UnixSocket(tokio::net::UnixStream),
    TcpSocket(tokio::net::TcpStream),
    File(tokio::fs::File),
}

impl AsyncRead for BackingIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::UnixSocket(ref mut s) => AsyncRead::poll_read(Pin::new(s), cx, buf),
            Self::TcpSocket(ref mut s) => AsyncRead::poll_read(Pin::new(s), cx, buf),
            Self::File(ref mut f) => AsyncRead::poll_read(Pin::new(f), cx, buf),
        }
    }
}

impl AsyncWrite for BackingIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::UnixSocket(ref mut s) => AsyncWrite::poll_write(Pin::new(s), cx, buf),
            Self::TcpSocket(ref mut s) => AsyncWrite::poll_write(Pin::new(s), cx, buf),
            Self::File(ref mut f) => AsyncWrite::poll_write(Pin::new(f), cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::UnixSocket(ref mut s) => AsyncWrite::poll_flush(Pin::new(s), cx),
            Self::TcpSocket(ref mut s) => AsyncWrite::poll_flush(Pin::new(s), cx),
            Self::File(ref mut f) => AsyncWrite::poll_flush(Pin::new(f), cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match self.get_mut() {
            Self::UnixSocket(ref mut s) => AsyncWrite::poll_shutdown(Pin::new(s), cx),
            Self::TcpSocket(ref mut s) => AsyncWrite::poll_shutdown(Pin::new(s), cx),
            Self::File(ref mut f) => AsyncWrite::poll_shutdown(Pin::new(f), cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        match self.get_mut() {
            Self::UnixSocket(ref mut s) => AsyncWrite::poll_write_vectored(Pin::new(s), cx, bufs),
            Self::TcpSocket(ref mut s) => AsyncWrite::poll_write_vectored(Pin::new(s), cx, bufs),
            Self::File(ref mut f) => AsyncWrite::poll_write_vectored(Pin::new(f), cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::UnixSocket(ref s) => AsyncWrite::is_write_vectored(s),
            Self::TcpSocket(ref s) => AsyncWrite::is_write_vectored(s),
            Self::File(ref f) => AsyncWrite::is_write_vectored(f),
        }
    }
}

impl BackingIo {
    pub fn from_value_and_raw_fd(value: Value, raw_fd: RawFd) -> Result<Self, Error> {
        let classname = unsafe { value.classname() };

        match classname.as_ref() {
            "UNIXSocket" => {
                let std_file = unsafe { std::os::unix::net::UnixStream::from_raw_fd(raw_fd) };
                let io = tokio::net::UnixStream::from_std(std_file);

                let io = io.map_err(|e| {
                    new_base_error!(
                        "Could not create tokio::net::UnixStream from RawFd {}: {}",
                        raw_fd,
                        e
                    )
                })?;

                Ok(Self::UnixSocket(io))
            }
            "IO" | "File" => {
                let io = unsafe { tokio::fs::File::from_raw_fd(raw_fd) };

                Ok(Self::File(io))
            }
            "Socket" => {
                let std_file = unsafe { std::net::TcpStream::from_raw_fd(raw_fd) };
                let io = tokio::net::TcpStream::from_std(std_file);
                let io = io.map_err(|e| {
                    new_base_error!(
                        "Could not create tokio::net::TcpStream from RawFd {}: {}",
                        raw_fd,
                        e
                    )
                })?;

                Ok(Self::TcpSocket(io))
            }
            _ => Err(new_base_error!(
                "Could not create BackingIo from value {:?} and RawFd {}",
                value,
                raw_fd
            )),
        }
    }
}

impl AsRawFd for BackingIo {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::UnixSocket(socket) => socket.as_raw_fd(),
            Self::TcpSocket(socket) => socket.as_raw_fd(),
            Self::File(file) => file.as_raw_fd(),
        }
    }
}

impl std::fmt::Display for BackingIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnixSocket(_) => write!(f, "UNIXSocket"),
            Self::TcpSocket(_) => write!(f, "TCPSocket"),
            Self::File(_) => write!(f, "File"),
        }
    }
}

#[must_use]
#[derive(Debug)]
pub struct NonBlockGuard {
    raw_fd: RawFd,
    old_flags: OFlag,
}

impl NonBlockGuard {
    fn new(fd: RawFd) -> Result<Self, Error> {
        let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| {
            new_base_error!("Could not get file descriptor flags for {}: {}", fd, e)
        })?;
        let old_flags = OFlag::from_bits_truncate(flags);
        let new_flags = old_flags | OFlag::O_NONBLOCK;

        fcntl(fd, FcntlArg::F_SETFL(new_flags)).map_err(|e| {
            new_base_error!("Could not set file descriptor flags for {}: {}", fd, e)
        })?;

        Ok(Self {
            raw_fd: fd,
            old_flags,
        })
    }
}

impl Drop for NonBlockGuard {
    fn drop(&mut self) {
        let _ = fcntl(self.raw_fd, FcntlArg::F_SETFL(self.old_flags));
    }
}

#[derive(Debug)]
pub struct RubyIo {
    value: Value,
    pub async_fd: Option<AsyncFd<BackingIo>>,
}

impl RubyIo {
    pub fn new_with_interest(value: Value, interest: Interest) -> Result<Self, Error> {
        let _io: Value = intern::class::io().funcall(intern::id::try_convert(), (value,))?;
        let fileno: RawFd = value.funcall(intern::id::fileno(), ())?;
        let backing_io = BackingIo::from_value_and_raw_fd(value, fileno)?;
        let async_fd = AsyncFd::with_interest(backing_io, interest).map_err(|e| {
            new_base_error!("Could not create AsyncFd from RawFd {}: {}", fileno, e)
        })?;

        Ok(Self {
            value,
            async_fd: Some(async_fd),
        })
    }

    pub fn with_nonblock(&self) -> Result<NonBlockGuard, Error> {
        let fd = self.async_fd();
        let raw = fd.get_ref().as_raw_fd();
        NonBlockGuard::new(raw)
    }

    pub async fn ready(
        &self,
        interest: Interest,
    ) -> Result<(Value, AsyncFdReadyGuard<'_, BackingIo>), Error> {
        let guard = {
            let span = tracing::span!(
                tracing::Level::DEBUG,
                "waiting_for_interest",
                ?self,
                ?interest
            );
            let _ = span.enter();
            self.async_fd().ready(interest).await.map_err(|e| {
                new_base_error!(
                    "Could not wait for interest {:?} on RawFd {:?}: {}",
                    interest,
                    *self.async_fd(),
                    e
                )
            })
        }?;

        debug!("IO is ready for interest {:?}", interest);

        Ok((self.value, guard))
    }

    pub fn async_fd(&self) -> &AsyncFd<BackingIo> {
        self.async_fd.as_ref().expect("RubyIo async_fd is None")
    }

    pub fn async_fd_mut(&mut self) -> &mut AsyncFd<BackingIo> {
        self.async_fd.as_mut().expect("RubyIo async_fd is None")
    }
}

impl AsyncRead for RubyIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        let ruby_io = self.get_mut();
        let backing_io = ruby_io.async_fd_mut().get_mut();
        let backing_io = Pin::new(backing_io);
        backing_io.poll_read(cx, buf)
    }
}

impl AsyncWrite for RubyIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let ruby_io = self.get_mut();
        let backing_io = ruby_io.async_fd_mut().get_mut();
        let backing_io = Pin::new(backing_io);
        backing_io.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let ruby_io = self.get_mut();
        let backing_io = ruby_io.async_fd_mut().get_mut();
        let backing_io = Pin::new(backing_io);
        backing_io.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let ruby_io = self.get_mut();
        let backing_io = ruby_io.async_fd_mut().get_mut();
        let backing_io = Pin::new(backing_io);
        backing_io.poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        let ruby_io = self.get_mut();
        let backing_io = ruby_io.async_fd_mut().get_mut();
        let backing_io = Pin::new(backing_io);
        backing_io.poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        let backing_io = self.async_fd().get_ref();
        backing_io.is_write_vectored()
    }
}

impl IntoValue for RubyIo {
    fn into_value_with(self, _handle: &magnus::Ruby) -> Value {
        self.value
    }
}

impl Drop for RubyIo {
    fn drop(&mut self) {
        let async_fd = self.async_fd.take();
        if let Some(async_fd) = async_fd {
            let inner = async_fd.into_inner();
            // Ensure that the underlying file descriptor is not closed
            std::mem::forget(inner)
        }
    }
}
