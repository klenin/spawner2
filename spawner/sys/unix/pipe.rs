use crate::sys::IntoInner;
use crate::Result;

use nix::fcntl::{fcntl, flock, open, FcntlArg, FdFlag, FlockArg, OFlag};
use nix::sys::stat::Mode;
use nix::unistd::{close, pipe, read, write};

use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::path::Path;

pub struct PipeFd(pub RawFd);

pub struct ReadPipe {
    fd: PipeFd,
}

pub struct WritePipe {
    fd: PipeFd,
    is_file: bool,
}

pub fn create() -> Result<(ReadPipe, WritePipe)> {
    let (read_fd, write_fd) = pipe()?;
    Ok((
        ReadPipe {
            fd: PipeFd::new(read_fd)?,
        },
        WritePipe {
            fd: PipeFd::new(write_fd)?,
            is_file: false,
        },
    ))
}

impl ReadPipe {
    pub fn open<P: AsRef<Path>>(path: P, exclusive: bool) -> Result<Self> {
        let raw_fd = open(
            path.as_ref(),
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW,
            Mode::S_IRUSR | Mode::S_IRGRP,
        )?;
        if exclusive {
            flock(raw_fd, FlockArg::LockExclusive)?;
        }
        Ok(Self {
            fd: PipeFd::new(raw_fd)?,
        })
    }

    pub fn null() -> Result<Self> {
        Self::open("/dev/null", false)
    }
}

impl Read for ReadPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read(self.fd.0, buf).map_err(|_| io::Error::last_os_error())
    }
}

impl IntoInner<PipeFd> for ReadPipe {
    fn into_inner(self) -> PipeFd {
        self.fd
    }
}

impl WritePipe {
    pub fn open<P: AsRef<Path>>(path: P, exclusive: bool) -> Result<Self> {
        let raw_fd = open(
            path.as_ref(),
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_NOFOLLOW,
            Mode::S_IRUSR | Mode::S_IWUSR | Mode::S_IWGRP | Mode::S_IRGRP,
        )?;
        if exclusive {
            flock(raw_fd, FlockArg::LockExclusive)?;
        }
        Ok(Self {
            fd: PipeFd::new(raw_fd)?,
            is_file: true,
        })
    }

    pub fn null() -> Result<Self> {
        Self::open("/dev/null", false)
    }

    pub fn is_file(&self) -> bool {
        self.is_file
    }
}

impl Write for WritePipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write(self.fd.0, buf).map_err(|_| io::Error::last_os_error())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl IntoInner<PipeFd> for WritePipe {
    fn into_inner(self) -> PipeFd {
        self.fd
    }
}

impl PipeFd {
    fn new(fd: RawFd) -> Result<Self> {
        fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        Ok(Self(fd))
    }
}

impl Drop for PipeFd {
    fn drop(&mut self) {
        close(self.0).ok();
    }
}
