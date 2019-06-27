use crate::sys::pipe as imp;
use crate::sys::{FromInner, IntoInner};
use crate::Result;

use std::io::{self, Read, Write};
use std::path::Path;

/// A reference to the reading end of a pipe or to the file opened in read mode.
///
/// [`create`]: fn.create.html
/// [`ReadPipe::open`]: struct.ReadPipe.html#method.open
#[derive(Debug)]
pub struct ReadPipe(imp::ReadPipe);

/// A reference to the writing end of a pipe or to the file opened in write mode.
///
/// [`create`]: fn.create.html
/// [`WritePipe::open`]: struct.WritePipe.html#method.open
#[derive(Debug)]
pub struct WritePipe(imp::WritePipe);

/// Creates a new pipe returning the [`ReadPipe`] and [`WritePipe`] pair.
///
/// [`ReadPipe`]: struct.ReadPipe.html
/// [`WritePipe`]: struct.WritePipe.html
pub fn create() -> Result<(ReadPipe, WritePipe)> {
    let (r, w) = imp::create()?;
    Ok((ReadPipe(r), WritePipe(w)))
}

impl ReadPipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        imp::ReadPipe::open(path).map(Self)
    }

    pub fn null() -> Result<Self> {
        imp::ReadPipe::null().map(Self)
    }
}

impl IntoInner<imp::ReadPipe> for ReadPipe {
    fn into_inner(self) -> imp::ReadPipe {
        self.0
    }
}

impl FromInner<imp::ReadPipe> for ReadPipe {
    fn from_inner(inner: imp::ReadPipe) -> Self {
        Self(inner)
    }
}

impl Read for ReadPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl WritePipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        imp::WritePipe::open(path).map(Self)
    }

    pub fn null() -> Result<Self> {
        imp::WritePipe::null().map(Self)
    }
}

impl IntoInner<imp::WritePipe> for WritePipe {
    fn into_inner(self) -> imp::WritePipe {
        self.0
    }
}

impl FromInner<imp::WritePipe> for WritePipe {
    fn from_inner(inner: imp::WritePipe) -> Self {
        Self(inner)
    }
}

impl Write for WritePipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
