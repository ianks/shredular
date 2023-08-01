use std::{ffi::c_void, mem::MaybeUninit};

use magnus::{
    exception::type_error,
    rb_sys::{protect, AsRawValue},
    Error, IntoValue, RTypedData, TryConvert, Value,
};

#[derive(Debug, Clone, Copy)]
pub struct RubyIoBuffer(RTypedData);

impl RubyIoBuffer {
    pub fn as_slice(&self) -> Result<&[u8], Error> {
        let mut base = MaybeUninit::<*const c_void>::uninit();
        let mut size = MaybeUninit::<rb_sys::size_t>::uninit();

        protect(|| {
            unsafe {
                rb_sys::rb_io_buffer_get_bytes_for_reading(
                    self.0.as_raw(),
                    base.as_mut_ptr(),
                    size.as_mut_ptr(),
                )
            };
            rb_sys::Qnil as _
        })?;

        Ok(unsafe { std::slice::from_raw_parts(base.assume_init() as _, size.assume_init() as _) })
    }

    pub fn as_mut_slice(&self) -> Result<&mut [u8], Error> {
        let mut base = MaybeUninit::<*mut c_void>::uninit();
        let mut size = MaybeUninit::<rb_sys::size_t>::uninit();

        protect(|| {
            unsafe {
                rb_sys::rb_io_buffer_get_bytes_for_writing(
                    self.0.as_raw(),
                    base.as_mut_ptr(),
                    size.as_mut_ptr(),
                )
            };
            rb_sys::Qnil as _
        })?;

        Ok(unsafe {
            std::slice::from_raw_parts_mut(base.assume_init() as _, size.assume_init() as _)
        })
    }
}

impl TryConvert for RubyIoBuffer {
    fn try_convert(value: Value) -> Result<Self, Error> {
        let typed_data = RTypedData::from_value(value).ok_or_else(|| {
            let inspected = unsafe { value.classname() };
            Error::new(
                type_error(),
                format!("expected an IO::Buffer, got {inspected}"),
            )
        })?;

        if unsafe { typed_data.classname() }.as_ref() != "IO::Buffer" {
            let inspected = unsafe { value.classname() };

            return Err(Error::new(
                type_error(),
                format!("expected an IO::Buffer, got {inspected}"),
            ));
        }

        Ok(Self(typed_data))
    }
}

impl IntoValue for RubyIoBuffer {
    fn into_value_with(self, handle: &magnus::Ruby) -> Value {
        self.0.into_value_with(handle)
    }
}

#[cfg(test)]
mod tests {
    use crate::fiber::Fiber;

    use super::*;
    use magnus::QNIL;
    use rb_sys_test_helpers::ruby_test;

    #[ruby_test]
    fn test_it_fails_to_convert_non_io_buffer() {
        let nil_value = *QNIL;
        let error = RubyIoBuffer::try_convert(nil_value).unwrap_err();

        assert_eq!(
            error.to_string(),
            "TypeError: expected an IO::Buffer, got NilClass"
        );

        let fiber_value = Fiber::current().as_value();
        let error = RubyIoBuffer::try_convert(fiber_value).unwrap_err();

        assert_eq!(
            "TypeError: expected an IO::Buffer, got Fiber",
            error.to_string(),
        );
    }

    #[ruby_test]
    fn test_it_converts_io_buffer() {
        let io_buffer = eval!("IO::Buffer.new").unwrap();
        let io_buffer = RubyIoBuffer::try_convert(io_buffer).unwrap();

        assert_eq!(
            "IO::Buffer",
            unsafe { io_buffer.into_value().classname() }.as_ref(),
        );
    }

    #[ruby_test]
    fn test_as_mut_slice() {
        let io_buffer = eval!("IO::Buffer.new").unwrap();
        let io_buffer = RubyIoBuffer::try_convert(io_buffer).unwrap();
        let slice = io_buffer.as_mut_slice().unwrap();

        assert_eq!(&[0; 65536], slice);
    }

    #[ruby_test]
    fn test_as_slice() {
        let io_buffer = eval!("IO::Buffer.for('foo')").unwrap();
        let io_buffer = RubyIoBuffer::try_convert(io_buffer).unwrap();
        let slice = io_buffer.as_slice().unwrap();

        assert_eq!(b"foo", slice);
    }
}
