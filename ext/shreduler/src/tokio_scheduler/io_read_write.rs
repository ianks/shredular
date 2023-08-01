use super::prelude::*;
use crate::{io_buffer::RubyIoBuffer, new_base_error, ruby_io::RubyIo};
use magnus::TryConvert;
use std::os::fd::RawFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Interest};

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

            // Prepare the buffer
            let mut buf = vec![0; minimum_length_to_read];
            let mut total_read = 0;

            while total_read < minimum_length_to_read {
                match io.read(&mut buf[total_read..]).await {
                    Ok(amt) => {
                        debug!(?amt, ?io, "Finished read from IO");
                        total_read += amt;
                        if amt == 0 || minimum_length_to_read == 0 {
                            break; // EOF reached or IO not ready, break the loop
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            // If the IO is not ready, wait until it is.
                            let (_, mut guard) =
                                io.ready(Interest::READABLE).await.map_err(|e| {
                                    new_base_error!(
                                        "Could not wait for interest {:?} on RawFd {}: {}",
                                        Interest::READABLE,
                                        *io.async_fd().get_ref(),
                                        e
                                    )
                                })?;
                            guard.clear_ready();
                            continue;
                        } else {
                            debug!(?e, ?io, "Error reading from IO");
                            return Ok::<_, Error>((-nix::errno::errno() as isize, buf));
                        }
                    }
                }
            }

            Ok::<_, Error>((total_read as isize, buf))
        };

        let future = Self::with_timeout("io_read", timeout, async move {
            let (amt, buf) = future.await?;
            buffer.set_string(buf)?;
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
        mut minimum_length_to_be_written: usize,
        timeout: TimeoutDuration,
    ) -> Result<Value, magnus::Error> {
        let timeout = if TimeoutDuration::is_zero(timeout) {
            None
        } else {
            Some(timeout)
        };

        let mut io = RubyIo::new_with_interest(io, Interest::WRITABLE)?;
        debug!(?io.async_fd, ?minimum_length_to_be_written, ?buffer, "IO write called");
        let buffer = RubyIoBuffer::try_convert(buffer)?;

        if minimum_length_to_be_written == 0 {
            minimum_length_to_be_written = buffer.size()?.to_usize()?;
        }
        let data = buffer.get_string(minimum_length_to_be_written)?;

        let future = async move {
            let _nonblock = io.with_nonblock()?;

            let mut total_written = 0;
            let data_slice = unsafe { data.as_slice() };

            while total_written < minimum_length_to_be_written {
                match io.write(&data_slice[total_written..]).await {
                    Ok(amt) => {
                        debug!(?amt, ?io, "Finished write to IO");
                        total_written += amt;
                        if amt == 0 {
                            break; // IO not ready or EOF reached, break the loop
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            // If the IO is not ready, wait until it is.
                            let (_, mut guard) =
                                io.ready(Interest::WRITABLE).await.map_err(|e| {
                                    new_base_error!(
                                        "Could not wait for interest {:?} on RawFd {}: {}",
                                        Interest::WRITABLE,
                                        *io.async_fd().get_ref(),
                                        e
                                    )
                                })?;

                            guard.clear_ready();
                            continue;
                        } else {
                            debug!(?e, ?io, "Error writing to IO");
                            return Ok::<_, Error>(-nix::errno::errno() as isize);
                        }
                    }
                }
            }

            Ok::<_, Error>(total_written as isize)
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
