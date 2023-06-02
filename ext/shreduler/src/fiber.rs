use magnus::{
    rb_sys::{protect, AsRawValue, FromRawValue},
    ArgList, RString, TryConvert, Value,
};
use rb_sys::{rb_fiber_transfer, rb_fiber_yield};

pub use self::state::{Running, State, Suspended, Terminated};

mod state {
    /// Marker trait for the different states of a fiber.
    pub trait State {}

    /// The fiber is running.
    #[derive(Clone, Copy, Debug)]
    pub struct Running;
    impl State for Running {}

    /// The fiber is suspended.
    #[derive(Clone, Copy, Debug)]
    pub struct Suspended;
    impl State for Suspended {}

    /// The fiber is terminated.
    #[derive(Clone, Copy, Debug)]
    pub struct Terminated;
    impl State for Terminated {}
}

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Fiber<S: state::State>(Value, std::marker::PhantomData<S>);

impl Fiber<Running> {
    /// The currently active Ruby fiber.
    pub fn current() -> Self {
        let marker = std::marker::PhantomData::<Running>;
        unsafe { Self(Value::from_raw(rb_sys::rb_fiber_current()), marker) }
    }

    /// Yields the control back to the point where the current fiber was resumed.
    /// The passed objects would be the return value of rb_fiber_resume. This fiber
    /// then suspends its execution until next time it is resumed.
    pub fn suspend<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Fiber<Suspended>, magnus::Error> {
        let args = args.into_arg_list();
        let args = args.as_ref();
        let fiber = protect(|| unsafe { rb_fiber_yield(args.len() as _, args.as_ptr() as _) })?;

        Ok(Fiber(
            unsafe { Value::from_raw(fiber) },
            std::marker::PhantomData::<Suspended>,
        ))
    }

    /// Transfer control to another fiber, resuming it from where it last stopped or
    /// starting it if it was not resumed before. The calling fiber will be
    /// suspended much like in a call to ::yield. You need to require 'fiber' before
    /// using this method.
    ///
    /// The fiber which receives the transfer call is treats it much like a resume
    /// call. Arguments passed to transfer are treated like those passed to resume.
    ///
    /// You cannot call resume on a fiber that has been transferred to. If you call
    /// transfer on a fiber, and later call resume on the the fiber, a FiberError
    /// will be raised. Once you call transfer on a fiber, the only way to resume
    /// processing the fiber is to call transfer on it again.
    pub fn transfer<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Value, magnus::Error> {
        let args = args.into_arg_list();
        let args = args.as_ref();
        let fiber = protect(|| unsafe {
            rb_fiber_transfer(self.0.as_raw(), args.len() as _, args.as_ptr() as _)
        })?;

        Ok(unsafe { Value::from_raw(fiber) })
    }
}

impl Fiber<Suspended> {
    /// Resumes the execution of the passed fiber, either from the point at
    /// which the last rb_fiber_yield was called if any, or at the beginning of
    /// the fiber body if it is the first call to this function.
    ///
    /// Other arguments are passed into the fiber's body, either as return
    /// values of rb_fiber_yield in case it switches to there, or as the block
    /// parameter of the fiber body if it switches to the beginning of the
    /// fiber.
    ///
    /// The return value of this function is either the value passed to previous
    /// rb_fiber_yield call, or the ultimate evaluated value of the entire fiber
    /// body if the execution reaches the end of it.
    ///
    /// When an exception happens inside of a fiber it propagates to this
    /// function.
    pub fn resume<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Value, magnus::Error> {
        let args = args.into_arg_list();
        let args = args.as_ref();

        let fiber = protect(|| unsafe {
            rb_sys::rb_fiber_resume(self.0.as_raw(), args.len() as _, args.as_ptr() as _)
        })?;

        Ok(unsafe { Value::from_raw(fiber) })
    }

    /// Transfer control to another fiber, resuming it from where it last stopped or
    /// starting it if it was not resumed before. The calling fiber will be
    /// suspended much like in a call to ::yield. You need to require 'fiber' before
    /// using this method.
    ///
    /// The fiber which receives the transfer call is treats it much like a resume
    /// call. Arguments passed to transfer are treated like those passed to resume.
    ///
    /// You cannot call resume on a fiber that has been transferred to. If you call
    /// transfer on a fiber, and later call resume on the the fiber, a FiberError
    /// will be raised. Once you call transfer on a fiber, the only way to resume
    /// processing the fiber is to call transfer on it again.
    pub fn transfer<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Value, magnus::Error> {
        let args = args.into_arg_list();
        let args = args.as_ref();
        let fiber = protect(|| unsafe {
            rb_fiber_transfer(self.0.as_raw(), args.len() as _, args.as_ptr() as _)
        })?;

        Ok(unsafe { Value::from_raw(fiber) })
    }

    /// From inside of the fiber this would be seen as if rb_fiber_yield raised.
    ///
    /// This function does return in case the passed fiber gracefully handled
    /// the passed exception. But if it does not, the raised exception
    /// propagates out of the passed fiber; this function then does not return.
    ///
    /// Parameters are passed to rb_make_exception to create an exception
    /// object. See its document for what are allowed here.
    ///
    /// It is a failure to call this function against a fiber which is resuming,
    /// have never run yet, or has already finished running.
    pub fn raise(
        self,
        exception_class: magnus::ExceptionClass,
        message: String,
    ) -> Result<Value, magnus::Error> {
        let args = [*exception_class, *RString::new(&message)];

        let fiber = protect(|| unsafe {
            rb_sys::rb_fiber_raise(self.0.as_raw(), args.len() as _, args.as_ptr() as _)
        })?;

        Ok(unsafe { Value::from_raw(fiber) })
    }
}

impl<S: State> Fiber<S> {
    /// Transmutes the fiber so it can be interacted with as if it were suspended.
    pub unsafe fn as_suspended(&self) -> Fiber<Suspended> {
        Fiber(self.0, std::marker::PhantomData::<Suspended>)
    }
}

fn guard_is_fiber(value: Value) -> Result<Value, magnus::Error> {
    if !unsafe { rb_sys::rb_obj_is_fiber(value.as_raw()) == rb_sys::Qtrue.into() } {
        return Err(magnus::Error::new(
            magnus::exception::type_error(),
            format!("no implicit conversion of {} into Fiber", unsafe {
                value.classname()
            }),
        ));
    }

    Ok(value)
}

fn guard_is_alive(value: Value) -> Result<Value, magnus::Error> {
    if !unsafe { rb_sys::rb_fiber_alive_p(value.as_raw()) == rb_sys::Qtrue.into() } {
        return Err(magnus::Error::new(
            magnus::exception::arg_error(),
            "dead fiber is never suspended",
        ));
    }

    Ok(value)
}

fn guard_is_current(value: Value) -> Result<Value, magnus::Error> {
    if !Fiber::current().0.as_raw() == value.as_raw() {
        return Err(magnus::Error::new(
            magnus::exception::arg_error(),
            "not the current fiber",
        ));
    }

    Ok(value)
}

impl TryConvert for Fiber<Suspended> {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        guard_is_fiber(value)?;
        guard_is_alive(value)?;

        if guard_is_current(value).is_ok() {
            return Err(magnus::Error::new(
                magnus::exception::arg_error(),
                "current fiber cannot be suspended",
            ));
        }

        Ok(Self(value, std::marker::PhantomData::<Suspended>))
    }
}

impl TryConvert for Fiber<Terminated> {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        guard_is_fiber(value)?;

        if guard_is_alive(value).is_ok() {
            return Err(magnus::Error::new(
                magnus::exception::arg_error(),
                "dead fiber cannot be resumed",
            ));
        }

        Ok(Self(value, std::marker::PhantomData::<Terminated>))
    }
}

impl TryConvert for Fiber<Running> {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        guard_is_fiber(value)?;
        guard_is_alive(value)?;
        guard_is_current(value)?;

        Ok(Self(value, std::marker::PhantomData::<Running>))
    }
}

impl<S: State> std::hash::Hash for Fiber<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_raw().hash(state);
    }
}

impl<S: State> PartialEq for Fiber<S> {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_raw() == other.0.as_raw()
    }
}

impl<S: State> Eq for Fiber<S> {}

impl<S: State> PartialOrd for Fiber<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.as_raw().partial_cmp(&other.0.as_raw())
    }
}

impl<S: State> std::cmp::Ord for Fiber<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_raw().cmp(&other.0.as_raw())
    }
}
