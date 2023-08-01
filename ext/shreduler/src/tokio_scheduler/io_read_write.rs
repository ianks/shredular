use super::prelude::*;
use crate::{io_buffer::RubyIoBuffer, new_base_error, ruby_io::RubyIo};
use magnus::TryConvert;
use nix::errno;
use std::os::fd::RawFd;
use tokio::io::AsyncWrite;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Interest, ReadBuf};

impl TokioScheduler {
    /// Invoked by IO#read to read length bytes from io into a specified buffer
    /// (see IO::Buffer).
    ///
    /// The length argument is the “minimum length to be read”. If the IO buffer
    /// size is 8KiB, but the length is 1024 (1KiB), up to 8KiB might be read,
    /// but at least 1KiB will be. Generally, the only case where less data than
    /// length will be read is if there is an error reading the data.
    ///
    /// Specifying a length of 0 is valid and means try reading at least once
    /// and return any available data.
    ///
    /// Suggested implementation should try to read from io in a non-blocking
    /// manner and call io_wait if the io is not ready (which will yield control
    /// to other fibers).
    ///
    /// See IO::Buffer for an interface available to return data.
    ///
    /// Expected to return number of bytes read, or, in case of an error, -errno
    /// (negated number corresponding to system’s error code).
    ///
    /// The method should be considered experimental.
    #[tracing::instrument]
    pub fn io_read(
        &self,
        io: Value,
        buffer: RubyIoBuffer,
        minimum_length_to_read: usize,
        timeout: TimeoutDuration,
    ) -> Result<Value, magnus::Error> {
        let timeout = if TimeoutDuration::is_zero(timeout) {
            None
        } else {
            Some(timeout)
        };

        let mut io = RubyIo::new_with_interest(io, Interest::READABLE)?;

        let future = async move {
            let _nonblock = io.with_nonblock()?;
            let buf = buffer.as_mut_slice()?;
            let amount_to_read = if minimum_length_to_read == 0 {
                buf.len()
            } else {
                minimum_length_to_read
            };

            if minimum_length_to_read > buf.len() {
                return Err(new_base_error!(
                    "Cannot read more than the buffer size: {} > {}",
                    minimum_length_to_read,
                    buf.len()
                ));
            }

            match read_at_least(&mut io, buf, amount_to_read).await {
                Ok(amt) => {
                    debug!(?amt, ?io, "Finished read from IO");
                    Ok::<_, Error>(amt as isize)
                }
                Err(err) => {
                    debug!(?err, ?io, "Error reading from IO");
                    let errno = err
                        .raw_os_error()
                        .unwrap_or(errno::Errno::UnknownErrno as i32);
                    Ok::<_, Error>(-(errno as isize))
                }
            }
        };

        let future = Self::with_timeout("io_read", timeout, async move {
            let amt = future.await?;
            Ok(amt.into_value())
        });

        self.spawn_and_transfer(future)
    }

    /// Invoked by IO#write or IO::Buffer#write to write length bytes to io from
    /// from a specified buffer (see IO::Buffer).
    ///
    /// The minimum_length argument is the “minimum length to be written”. If
    /// the IO buffer size is 8KiB, but the length specified is 1024 (1KiB), at
    /// most 8KiB will be written, but at least 1KiB will be. Generally, the
    /// only case where less data than minimum_length will be written is if
    /// there is an error writing the data.
    ///
    /// Specifying a length of 0 is valid and means try writing at least once,
    /// as much data as possible.
    ///
    /// Suggested implementation should try to write to io in a non-blocking
    /// manner and call io_wait if the io is not ready (which will yield control
    /// to other fibers).
    ///
    /// See IO::Buffer for an interface available to get data from buffer
    /// efficiently.
    ///
    /// Expected to return number of bytes written, or, in case of an error,
    /// -errno (negated number corresponding to system’s error code).
    #[tracing::instrument]
    pub fn io_write(
        &self,
        io: Value,
        buffer: Value,
        minimum_length_to_be_written: usize,
        timeout: TimeoutDuration,
    ) -> Result<Value, magnus::Error> {
        let timeout = if TimeoutDuration::is_zero(timeout) {
            None
        } else {
            Some(timeout)
        };

        let mut io = RubyIo::new_with_interest(io, Interest::WRITABLE)?;
        let buffer = RubyIoBuffer::try_convert(buffer)?;

        let future = async move {
            let _nonblock = io.with_nonblock()?;
            let data = buffer.as_slice()?;

            let amount_to_write = if minimum_length_to_be_written == 0 {
                data.len()
            } else {
                minimum_length_to_be_written
            };

            if minimum_length_to_be_written > data.len() {
                return Err(new_base_error!(
                    "Cannot write more than the buffer size: {} > {}",
                    minimum_length_to_be_written,
                    data.len()
                ));
            }

            match write_at_least(&mut io, data, amount_to_write).await {
                Ok(amt) => {
                    debug!(?amt, ?io, "Finished write to IO");
                    Ok::<_, Error>(amt as isize)
                }
                Err(errno) => {
                    debug!(?errno, ?io, "Error writing to IO");
                    let errno = errno
                        .raw_os_error()
                        .unwrap_or(errno::Errno::UnknownErrno as i32);
                    Ok::<_, Error>(-(errno as isize))
                }
            }
        };

        let future = Self::with_timeout("io_write", timeout, async move {
            let amt = future.await?;
            Ok(amt.into_value())
        });

        self.spawn_and_transfer(future)
    }

    /// Reads data from an IO object into a buffer at a specified offset.
    #[tracing::instrument]
    pub fn io_pread(
        &self,
        io: RawFd,
        buffer: &mut [u8],
        from: usize,
        length: usize,
        offset: u64,
    ) -> Result<usize, Error> {
        crate::rtodo!("io_pread")
    }

    /// Writes data to an IO object from a buffer at a specified offset.
    #[tracing::instrument]
    pub fn io_pwrite(
        &self,
        io: RawFd,
        buffer: &[u8],
        from: usize,
        length: usize,
        offset: u64,
    ) -> Result<usize, Error> {
        crate::rtodo!("io_pwrite")
    }
}
pub async fn read_at_least<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut [u8],
    min: usize,
) -> Result<usize, std::io::Error> {
    assert!(buf.len() >= min);
    let mut total_read = 0;

    while total_read < min {
        let mut read_buf = ReadBuf::new(&mut buf[total_read..]);
        match reader.read_buf(&mut read_buf).await {
            Ok(just_read) => {
                if just_read == 0 {
                    // EOF reached
                    break;
                }
                total_read += just_read;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // The IO operation would block,
                // we'll need to wait until it's ready again before continuing
                tokio::task::yield_now().await;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(total_read)
}

#[tracing::instrument(skip(writer, buf))]
pub async fn write_at_least<W: AsyncWrite + Unpin>(
    writer: &mut W,
    buf: &[u8],
    min: usize,
) -> Result<usize, std::io::Error> {
    assert!(buf.len() >= min);
    let mut total_written = 0;

    while total_written < min {
        match writer.write(&buf[total_written..]).await {
            Ok(amt) => {
                debug!(?amt, "Wrote to IO");
                if amt == 0 {
                    break;
                }
                total_written += amt;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // The IO operation would block,
                // we'll need to wait until it's ready again before continuing
                tokio::task::yield_now().await;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(total_written)
}
