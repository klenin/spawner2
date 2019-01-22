use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::u32;

pub trait IsZero {
    fn is_zero(&self) -> bool;
}

macro_rules! impl_is_zero {
    ($($type:ident)*) => ($(
        impl IsZero for $type {
            fn is_zero(&self) -> bool {
                *self == 0
            }
        }
    )*)
}

pub trait IsMinusOne {
    fn is_minus_one(&self) -> bool;
}

macro_rules! impl_is_minus_one {
    ($($type:ident)*) => ($(
        impl IsMinusOne for $type {
            fn is_minus_one(&self) -> bool {
                *self == -1
            }
        }
    )*)
}

impl_is_zero!(i8 i16 i32 i64 isize u8 u16 u32 u64 usize);
impl_is_minus_one!(i8 i16 i32 i64 isize);

impl<T> IsZero for *const T {
    fn is_zero(&self) -> bool {
        self.is_null()
    }
}

impl<T> IsZero for *mut T {
    fn is_zero(&self) -> bool {
        self.is_null()
    }
}

impl IsMinusOne for u32 {
    fn is_minus_one(&self) -> bool {
        *self == u32::MAX
    }
}

/// returns last os error if the value is zero
pub fn ok_nonzero<T: IsZero>(v: T) -> io::Result<T> {
    if v.is_zero() {
        Err(io::Error::last_os_error())
    } else {
        Ok(v)
    }
}

/// returns last os error if the value is minus one
pub fn ok_neq_minus_one<T: IsMinusOne>(v: T) -> io::Result<T> {
    if v.is_minus_one() {
        Err(io::Error::last_os_error())
    } else {
        Ok(v)
    }
}

pub fn to_utf16<S: AsRef<OsStr>>(s: S) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}
