use crate::{Error, Result};

use nix::sys::mman::{mmap, munmap, MapFlags, ProtFlags};

use std::mem;
use std::ops;
use std::ptr;
use std::sync::Mutex;

pub struct SharedMem<T>(*mut Mutex<T>)
where
    T: Copy + Send;

impl<T> SharedMem<T>
where
    T: Copy + Send,
{
    pub fn alloc(v: T) -> Result<Self> {
        unsafe {
            mmap(
                /*addr=*/ ptr::null_mut(),
                /*length=*/ mem::size_of::<Mutex<T>>(),
                /*prot=*/ ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                /*flags=*/ MapFlags::MAP_ANONYMOUS | MapFlags::MAP_SHARED,
                /*fd=*/ -1,
                /*offset=*/ 0,
            )
            .map(|ptr| {
                mem::forget(mem::replace(&mut *(ptr as *mut _), Mutex::new(v)));
                Self(ptr as *mut _)
            })
            .map_err(Error::from)
        }
    }
}

impl<T> Drop for SharedMem<T>
where
    T: Copy + Send,
{
    fn drop(&mut self) {
        unsafe {
            let _ = munmap(self.0 as *mut _, mem::size_of::<Mutex<T>>());
        }
    }
}

impl<T> ops::Deref for SharedMem<T>
where
    T: Copy + Send,
{
    type Target = Mutex<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0 }
    }
}

impl<T> ops::DerefMut for SharedMem<T>
where
    T: Copy + Send,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.0 }
    }
}
