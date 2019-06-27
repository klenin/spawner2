use crate::sys::IntoInner;
use crate::{Error, Result};

use nix::fcntl::{fcntl, open, FcntlArg, FdFlag, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, pipe, read, write};

use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::path::Path;

#[derive(Debug)]
pub struct PipeFd(RawFd);

#[derive(Debug)]
pub struct ReadPipe(PipeFd);

#[derive(Debug)]
pub struct WritePipe(PipeFd);

pub fn create() -> Result<(ReadPipe, WritePipe)> {
    let (read_fd, write_fd) = pipe()?;
    Ok((
        ReadPipe(PipeFd::new(read_fd)?),
        WritePipe(PipeFd::new(write_fd)?),
    ))
}

impl PipeFd {
    fn new(fd: RawFd) -> Result<Self> {
        fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        Ok(Self(fd))
    }

    pub fn raw(&self) -> RawFd {
        self.0
    }
}

impl Drop for PipeFd {
    fn drop(&mut self) {
        close(self.0).ok();
    }
}

impl ReadPipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        open(
            path.as_ref(),
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW,
            Mode::S_IRUSR | Mode::S_IRGRP,
        )
        .map_err(Error::from)
        .and_then(PipeFd::new)
        .map(Self)
    }

    pub fn null() -> Result<Self> {
        Self::open("/dev/null")
    }

    fn raw(&self) -> RawFd {
        (self.0).0
    }
}

impl Read for ReadPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read(self.raw(), buf).map_err(|_| io::Error::last_os_error())
    }
}

impl IntoInner<PipeFd> for ReadPipe {
    fn into_inner(self) -> PipeFd {
        self.0
    }
}

impl WritePipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        open(
            path.as_ref(),
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_NOFOLLOW,
            Mode::S_IRUSR | Mode::S_IWUSR | Mode::S_IWGRP | Mode::S_IRGRP,
        )
        .map_err(Error::from)
        .and_then(PipeFd::new)
        .map(Self)
    }

    pub fn null() -> Result<Self> {
        Self::open("/dev/null")
    }

    fn raw(&self) -> RawFd {
        (self.0).0
    }
}

impl Write for WritePipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write(self.raw(), buf).map_err(|_| io::Error::last_os_error())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl IntoInner<PipeFd> for WritePipe {
    fn into_inner(self) -> PipeFd {
        self.0
    }
}
