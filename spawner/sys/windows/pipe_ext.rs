use crate::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::pipe as imp;
use crate::sys::FromInner;
use crate::Result;

use std::path::Path;

pub trait ReadPipeExt: Sized {
    fn lock<P: AsRef<Path>>(path: P) -> Result<Self>;
}

pub trait WritePipeExt: Sized {
    fn lock<P: AsRef<Path>>(path: P) -> Result<Self>;
}

impl ReadPipeExt for ReadPipe {
    fn lock<P: AsRef<Path>>(path: P) -> Result<Self> {
        imp::ReadPipe::lock(path).map(Self::from_inner)
    }
}

impl WritePipeExt for WritePipe {
    fn lock<P: AsRef<Path>>(path: P) -> Result<Self> {
        imp::WritePipe::lock(path).map(Self::from_inner)
    }
}
