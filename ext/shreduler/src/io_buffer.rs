use crate::intern;
use magnus::{
    exception::type_error, Error, Integer, IntoValue, RString, RTypedData, TryConvert, Value,
};

#[derive(Debug, Clone, Copy)]
pub struct RubyIoBuffer(RTypedData);

impl RubyIoBuffer {
    pub fn set_string<T: AsRef<[u8]>>(self, bytes: T) -> Result<Integer, Error> {
        let bytes = bytes.as_ref();
        let rstring = RString::from_slice(bytes);

        self.0.funcall_public(intern::id::set_string(), (rstring,))
    }

    pub fn get_string(self, offset: usize) -> Result<RString, Error> {
        self.0.funcall_public(intern::id::get_string(), (offset,))
    }

    pub fn size(self) -> Result<Integer, Error> {
        self.0.funcall_public(intern::id::size(), ())
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
    fn test_set_string_returns_length() {
        let io_buffer = eval!("IO::Buffer.new").unwrap();
        let io_buffer = RubyIoBuffer::try_convert(io_buffer).unwrap();

        let length = io_buffer.set_string("hello").unwrap();

        assert_eq!(5, length.to_usize().unwrap());
    }
}
