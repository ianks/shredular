use magnus::gc::register_mark_object;
use magnus::{ExceptionClass, Module};
use rb_sys::rb_during_gc;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI32, Ordering};

type Result<T> = std::result::Result<T, magnus::Error>;

/// `GcCell` is a mutable memory location that enforces borrow rules at runtime
/// and allows safely sharing mutable data between threads when the garbage
/// collector is running.
#[derive(Debug, Default)]
pub struct GcCell<T> {
    value: UnsafeCell<T>,
    read_count: AtomicI32,
    write_count: AtomicI32,
}

impl<T> GcCell<T> {
    /// Create a new `GcCell` containing `value`
    #[allow(dead_code)]
    pub fn new(value: T) -> Self {
        GcCell {
            value: UnsafeCell::new(value),
            read_count: Default::default(),
            write_count: Default::default(),
        }
    }

    /// Attempt to immutably borrow the cell's content, returns `None` if
    /// already borrowed.
    #[allow(dead_code)]
    pub fn try_borrow(&self) -> Result<GcRef<T>> {
        if self.write_count.load(Ordering::Acquire) > 0 {
            Err(Self::new_error("already mutably borrowed"))
        } else {
            self.read_count.fetch_add(1, Ordering::AcqRel);
            Ok(GcRef { gc_cell: self })
        }
    }

    /// Attempt to mutably borrow the cell's content, returns `None` if already
    /// borrowed.
    pub fn try_borrow_mut(&self) -> Result<GcRefMut<T>> {
        if self.read_count.load(Ordering::Acquire) > 0
            || self.write_count.load(Ordering::Acquire) > 0
        {
            Err(Self::new_error("already borrowed"))
        } else {
            self.write_count.fetch_add(1, Ordering::AcqRel);
            Ok(GcRefMut { gc_cell: self })
        }
    }

    /// Safely attempt to borrow the cell's content during garbage collection.
    /// Unsafe because it should only be called during GC.
    pub fn borrow_for_gc(&self) -> &T {
        assert!(
            unsafe { rb_during_gc() } == 1,
            "attempted to borrow_for_gc outside of GC"
        );
        unsafe { &*self.value.get() }
    }

    /// `PoisonedCellError` is raised when a `GcCell` is already borrowed
    fn error_class() -> ExceptionClass {
        *memoize!(ExceptionClass: {
          let klass = magnus::class::object().define_error("PoisonedCellError", Default::default()).unwrap();
          register_mark_object(*klass);
          klass
        })
    }

    fn new_error(message: &str) -> magnus::Error {
        magnus::Error::new(Self::error_class(), message.to_string())
    }
}

/// `GcRef` is a wrapper type for an immutably borrowed value from a
/// `GcCell<T>`.
pub struct GcRef<'a, T> {
    gc_cell: &'a GcCell<T>,
}

impl<T> Deref for GcRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_cell.value.get() }
    }
}

/// `GcRefMut` is a wrapper type for a mutably borrowed value from a
/// `GcCell<T>`.
pub struct GcRefMut<'a, T> {
    gc_cell: &'a GcCell<T>,
}

impl<T> Deref for GcRefMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_cell.value.get() }
    }
}

impl<T> DerefMut for GcRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.gc_cell.value.get() }
    }
}

impl<T> Drop for GcRef<'_, T> {
    fn drop(&mut self) {
        self.gc_cell.read_count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<T> Drop for GcRefMut<'_, T> {
    fn drop(&mut self) {
        self.gc_cell.write_count.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use super::*;
    use rb_sys_test_helpers::ruby_test;

    #[ruby_test]
    fn test_it_allows_multiple_immutable_borrows() {
        let cell = GcCell::new(1);
        let ref1 = cell.try_borrow().unwrap();
        let ref2 = cell.try_borrow().unwrap();
        assert_eq!(*ref1, 1);
        assert_eq!(*ref2, 1);
        drop(ref1);
        drop(ref2);
    }

    #[ruby_test]
    fn test_it_disallows_multiple_mut_borrows() {
        let cell = GcCell::new(1);
        let ref1 = cell.try_borrow_mut().unwrap();
        assert!(cell.try_borrow_mut().is_err());
        drop(ref1);
        let ref2 = cell.try_borrow_mut().unwrap();
        assert!(cell.try_borrow_mut().is_err());
        drop(ref2);
    }

    #[ruby_test]
    fn test_it_disallows_mixed_borrows() {
        let cell = GcCell::new(1);
        let ref1 = cell.try_borrow().unwrap();
        assert!(cell.try_borrow_mut().is_err());
        drop(ref1);
        let ref2 = cell.try_borrow_mut().unwrap();
        assert!(cell.try_borrow().is_err());
        drop(ref2);
    }

    #[ruby_test]
    fn test_it_allows_borrow_after_drop() {
        let cell = GcCell::new(1);
        {
            let _ref1 = cell.try_borrow().unwrap();
        }
        assert!(cell.try_borrow().is_ok());
    }

    #[ruby_test]
    fn test_it_allows_mutable_borrow_after_drop() {
        let cell = GcCell::new(1);
        {
            let _ref1 = cell.try_borrow_mut().unwrap();
        }
        assert!(cell.try_borrow_mut().is_ok());
    }
}
