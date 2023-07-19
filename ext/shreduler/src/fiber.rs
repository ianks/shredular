use std::marker::PhantomData;

use magnus::{
    block::Proc,
    class::object,
    exception::{runtime_error, type_error},
    rb_sys::{protect, AsRawValue, FromRawValue},
    ArgList, Error, IntoValue, Module, RClass, RHash, RString, Symbol, Value,
};
use rb_sys::{rb_fiber_transfer, rb_fiber_yield};

pub use self::state::{Running, State, Suspended, Terminated, Unknown};

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

    /// Fiber in unknown state.
    #[derive(Clone, Copy, Debug)]
    pub struct Unknown;
    impl State for Unknown {}
}

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Fiber<S: state::State>(Value, PhantomData<S>);

impl Fiber<Running> {
    /// The currently active Ruby fiber.
    pub fn current() -> Self {
        let marker = PhantomData::<Running>;
        unsafe { Self(Value::from_raw(rb_sys::rb_fiber_current()), marker) }
    }

    /// Yields the control back to the point where the current fiber was resumed.
    /// The passed objects would be the return value of rb_fiber_resume. This fiber
    /// then suspends its execution until next time it is resumed.
    #[allow(dead_code)]
    pub fn suspend<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Fiber<Suspended>, Error> {
        let args = args.into_arg_list();
        let args = args.as_ref();
        let fiber = protect(|| unsafe { rb_fiber_yield(args.len() as _, args.as_ptr() as _) })?;

        Ok(Fiber(
            unsafe { Value::from_raw(fiber) },
            PhantomData::<Suspended>,
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
    #[allow(dead_code)]
    pub fn transfer<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Value, Error> {
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
    #[allow(dead_code)]
    pub fn resume<O: AsRef<[Value]>, A: ArgList<Output = O>>(
        self,
        args: A,
    ) -> Result<Value, Error> {
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
    ) -> Result<Value, Error> {
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
    #[allow(dead_code)]
    pub fn raise(self, error: Error) -> Result<Value, Error> {
        let result = match error {
            Error::Error(klass, message) => {
                let message = RString::new(message.as_ref());
                let args = [*klass, *message];
                protect(|| unsafe { rb_sys::rb_fiber_raise(self.as_raw(), 2, args.as_ptr() as _) })
            }
            Error::Exception(e) => protect(|| unsafe {
                rb_sys::rb_fiber_raise(self.as_raw(), 1, &e.as_raw() as *const _ as _)
            }),
            Error::Jump(_) => unreachable!("jump error should not be returned"),
        };

        result.map(|fiber| unsafe { Value::from_raw(fiber) })
    }
}

impl Fiber<Unknown> {
    pub fn check_suspended(&self) -> Result<Fiber<Suspended>, Error> {
        if !self.is_alive() {
            return Err(Error::new(runtime_error(), "fiber not alive"));
        }

        if self.is_current() {
            return Err(Error::new(runtime_error(), "fiber active"));
        }

        Ok(Fiber(self.0, PhantomData::<Suspended>))
    }
}

impl<S: State> Fiber<S> {
    /// Creates a new fiber from a block.
    pub fn new(block: Proc) -> Result<Fiber<Suspended>, Error> {
        let fiber = fiber_class().funcall_with_block("new", (), block)?;

        Ok(Fiber(fiber, PhantomData::<Suspended>))
    }

    pub fn new_nonblocking(block: Proc) -> Result<Fiber<Suspended>, Error> {
        let kwargs = RHash::new();
        kwargs.aset(Symbol::new("blocking"), false)?;
        let fiber = fiber_class().funcall_with_block("new", (kwargs,), block)?;

        Ok(Fiber(fiber, PhantomData::<Suspended>))
    }

    pub fn spawn_nonblocking(block: Proc) -> Result<Fiber<Suspended>, Error> {
        let fiber = Fiber::<Suspended>::new_nonblocking(block)?;
        fiber.transfer(())?;

        Ok(fiber)
    }

    #[allow(dead_code)]
    pub fn from_value(value: Value) -> Result<Fiber<Unknown>, Error> {
        if !unsafe { rb_sys::rb_obj_is_fiber(value.as_raw()) == rb_sys::Qtrue.into() } {
            return Err(Error::new(
                type_error(),
                format!("no implicit conversion of {} into Fiber", unsafe {
                    value.classname()
                }),
            ));
        }

        Ok(Fiber(value, PhantomData::<Unknown>))
    }

    #[allow(dead_code)]
    unsafe fn from_value_unchecked<T: State>(value: Value) -> Fiber<T> {
        Fiber(value, PhantomData::<T>)
    }
    /// Transmutes the fiber so it can be interacted with as if it were suspended.
    pub unsafe fn as_suspended(&self) -> Fiber<Suspended> {
        Fiber(self.0, PhantomData::<Suspended>)
    }

    /// Transmute this fiber to be in an "unchecked" state, so it can be used
    /// without presumption.
    pub fn as_unchecked(&self) -> Fiber<Unknown> {
        Fiber(self.0, PhantomData::<Unknown>)
    }

    pub fn is_current(&self) -> bool {
        Fiber::current().as_raw() == self.0.as_raw()
    }

    pub fn is_alive(&self) -> bool {
        unsafe { rb_sys::rb_fiber_alive_p(self.0.as_raw()) == rb_sys::Qtrue.into() }
    }
}

impl<S: State> std::ops::Deref for Fiber<S> {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn fiber_class() -> RClass {
    *magnus::memoize!(RClass: {
        let c: RClass = object().const_get("Fiber").expect("Fiber class not found");
        magnus::gc::register_mark_object(*c);
        c
    })
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

impl<S: State> IntoValue for Fiber<S> {
    fn into_value_with(self, _handle: &magnus::Ruby) -> Value {
        self.0
    }
}
