use crate::sys::windows::helpers::{cvt, to_utf16, Handle};
use crate::sys::IntoInner;
use crate::{Error, Result};

use winapi::shared::minwindef::{DWORD, TRUE};
use winapi::um::fileapi::{CreateFileW, ReadFile, WriteFile, CREATE_ALWAYS, OPEN_EXISTING};
use winapi::um::handleapi::{SetHandleInformation, INVALID_HANDLE_VALUE};
use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
use winapi::um::namedpipeapi::CreatePipe;
use winapi::um::winbase::HANDLE_FLAG_INHERIT;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE,
};

use std::io::{self, Read, Write};
use std::mem;
use std::path::Path;
use std::ptr;

pub struct ReadPipe {
    handle: Handle,
}

pub struct WritePipe {
    handle: Handle,
    is_file: bool,
}

pub fn create() -> Result<(ReadPipe, WritePipe)> {
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        bInheritHandle: TRUE,
        lpSecurityDescriptor: ptr::null_mut(),
    };

    let mut read_handle = INVALID_HANDLE_VALUE;
    let mut write_handle = INVALID_HANDLE_VALUE;
    unsafe {
        cvt(CreatePipe(
            &mut read_handle,
            &mut write_handle,
            &mut attrs,
            0,
        ))?;
    }

    Ok((
        ReadPipe {
            handle: Handle(read_handle),
        },
        WritePipe {
            handle: Handle(write_handle),
            is_file: false,
        },
    ))
}

impl ReadPipe {
    pub fn open<P: AsRef<Path>>(path: P, exclusive: bool) -> Result<Self> {
        Ok(Self {
            handle: open(path, GENERIC_READ, OPEN_EXISTING, exclusive)?,
        })
    }

    pub fn null() -> Result<Self> {
        Self::open("nul", false)
    }
}

impl IntoInner<Handle> for ReadPipe {
    fn into_inner(self) -> Handle {
        self.handle
    }
}

impl Read for ReadPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes_read: DWORD = 0;
        unsafe {
            cvt(ReadFile(
                self.handle.0,
                mem::transmute(buf.as_mut_ptr()),
                buf.len() as DWORD,
                &mut bytes_read,
                ptr::null_mut(),
            ))
            .map_err(|_| io::Error::last_os_error())?;
        }
        Ok(bytes_read as usize)
    }
}

impl WritePipe {
    pub fn open<P: AsRef<Path>>(path: P, exclusive: bool) -> Result<Self> {
        Ok(Self {
            handle: open(path, GENERIC_WRITE, CREATE_ALWAYS, exclusive)?,
            is_file: true,
        })
    }

    pub fn null() -> Result<Self> {
        Ok(Self {
            handle: open("nul", GENERIC_WRITE, OPEN_EXISTING, false)?,
            is_file: false,
        })
    }

    pub fn is_file(&self) -> bool {
        self.is_file
    }
}

impl IntoInner<Handle> for WritePipe {
    fn into_inner(self) -> Handle {
        self.handle
    }
}

impl Write for WritePipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes_written: DWORD = 0;
        unsafe {
            cvt(WriteFile(
                self.handle.0,
                mem::transmute(buf.as_ptr()),
                buf.len() as DWORD,
                &mut bytes_written,
                ptr::null_mut(),
            ))
            .map_err(|_| io::Error::last_os_error())?;
        }
        Ok(bytes_written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn open<P: AsRef<Path>>(
    path: P,
    access: DWORD,
    creation_disposition: DWORD,
    exclusive: bool,
) -> Result<Handle> {
    let handle = unsafe {
        Handle(CreateFileW(
            /*lpFileName=*/ to_utf16(path.as_ref()).as_mut_ptr(),
            /*dwDesiredAccess=*/ access,
            /*dwShareMode=*/
            match exclusive {
                true => 0,
                false => FILE_SHARE_READ | FILE_SHARE_WRITE,
            },
            /*lpSecurityAttributes=*/ ptr::null_mut(),
            /*dwCreationDisposition=*/ creation_disposition,
            /*dwFlagsAndAttributes=*/ FILE_ATTRIBUTE_NORMAL,
            /*hTemplateFile=*/ ptr::null_mut(),
        ))
    };

    if handle.0 == INVALID_HANDLE_VALUE {
        return Err(Error::last_os_error());
    }

    unsafe {
        cvt(SetHandleInformation(
            handle.0,
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        ))
        .map_err(Error::from)
        .map(|_| handle)
    }
}
