use std::os::fd::RawFd;

use super::prelude::*;

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
        io: RawFd,
        buffer: &mut [u8],
        minimum_length_to_read: usize,
    ) -> Result<usize, magnus::Error> {
        crate::rtodo!("io_read")
    }

    /// Writes data to an IO object from a buffer.
    #[tracing::instrument]
    pub fn io_write(&self, io: RawFd, buffer: &[u8], length: usize) -> Result<usize, Error> {
        crate::rtodo!("io_write")
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
