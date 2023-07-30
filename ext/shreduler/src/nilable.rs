use magnus::{TryConvert, Value};

#[derive(Debug, Clone, Copy)]
pub enum Nilable<T: TryConvert> {
    Value(T),
    Nil,
}

impl<T: TryConvert> TryConvert for Nilable<T> {
    fn try_convert(value: Value) -> Result<Self, magnus::Error> {
        if value.is_nil() {
            Ok(Self::Nil)
        } else {
            Ok(Self::Value(T::try_convert(value)?))
        }
    }
}

#[allow(dead_code)]
impl<T: TryConvert> Nilable<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Nil => None,
        }
    }

    pub fn is_some(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::Nil)
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }
}
