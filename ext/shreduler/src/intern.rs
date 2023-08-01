pub mod id {
    use magnus::value::Id;

    pub fn fileno() -> Id {
        *memoize!(Id: Id::new("fileno"))
    }

    pub fn try_convert() -> Id {
        *memoize!(Id: Id::new("try_convert"))
    }
}

pub mod class {
    use magnus::{class::object, gc::register_mark_object, Module, RClass, RModule};

    pub fn io() -> RClass {
        *memoize!(RClass: {
          let io: RClass = object().const_get("IO").unwrap();
          register_mark_object(io);
          io
        })
    }

    pub fn timeout() -> RModule {
        *memoize!(RModule: {
          let timeout: RModule = object().const_get("Timeout").unwrap();
          register_mark_object(timeout);
          timeout
        })
    }
}

pub mod object {
    use magnus::{gc::register_mark_object, RArray};

    pub fn empty_array() -> RArray {
        *memoize!(RArray: {
          let array: RArray = RArray::new();
          register_mark_object(array);
          array
        })
    }
}
