use crate::{Error, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::u32;
use winapi::um::handleapi::CloseHandle;
use winapi::um::winnt::HANDLE;

pub struct Handle(pub HANDLE);
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

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

impl_is_zero!(i8 i16 i32 i64 isize u8 u16 u32 u64 usize);

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

/// Returns last os error if the value is zero.
pub fn cvt<T: IsZero>(v: T) -> Result<T> {
    if v.is_zero() {
        Err(Error::last_os_error())
    } else {
        Ok(v)
    }
}

pub fn to_utf16<S: AsRef<OsStr>>(s: S) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}
