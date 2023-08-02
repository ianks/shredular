use magnus::{
    exception::standard_error, gc::register_mark_object, ExceptionClass, Module, TypedData,
};

use crate::{intern, tokio_scheduler::TokioScheduler};

/// The base error class for the scheduler.
pub fn base_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("Error", standard_error()).unwrap();
        register_mark_object(*c);
        c
    })
}

/// Returns Timeout::Error.
pub fn timeout_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
      let c: ExceptionClass = intern::class::timeout().const_get("Error").unwrap();
        register_mark_object(*c);
        c
    })
}

/// Returns TokioScheduler::ReadyError.
pub fn ready_error() -> ExceptionClass {
    *memoize!(ExceptionClass: {
        let c = TokioScheduler::class().define_error("ReadyError", base_error()).unwrap();
        register_mark_object(*c);
        c
    })
}

/// New TokioScheduler::ReadyError.
#[macro_export]
macro_rules! new_ready_error {
    ($($arg:tt)*) => {
        magnus::Error::new($crate::errors::ready_error(), format!($($arg)*))
    };
}

/// A macro with creates an Err(magnus::Error) with a fmt message.
#[macro_export]
macro_rules! new_base_error {
    ($($arg:tt)*) => {
        magnus::Error::new($crate::errors::base_error(), format!($($arg)*))
    };
}

#[macro_export]
macro_rules! new_type_error {
    ($($arg:tt)*) => {
        magnus::Error::new(magnus::exception::type_error(), format!($($arg)*))
    };
}

#[macro_export]
macro_rules! new_timeout_error {
    ($($arg:tt)*) => {
        magnus::Error::new($crate::errors::timeout_error(), format!($($arg)*))
    };
}
