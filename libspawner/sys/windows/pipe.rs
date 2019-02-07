use crate::{Error, Result};
use std::io::{self, Read, Write};
use std::mem;
use std::path::Path;
use std::ptr;
use sys::windows::common::{ok_nonzero, to_utf16};
use winapi::shared::minwindef::{DWORD, TRUE};
use winapi::um::fileapi::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, CREATE_ALWAYS, OPEN_EXISTING,
};
use winapi::um::handleapi::{CloseHandle, SetHandleInformation, INVALID_HANDLE_VALUE};
use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
use winapi::um::namedpipeapi::CreatePipe;
use winapi::um::winbase::HANDLE_FLAG_INHERIT;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE,
};

pub struct ReadPipe {
    pub(crate) handle: HANDLE,
}

pub struct WritePipe {
    pub(crate) handle: HANDLE,
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
        ok_nonzero(CreatePipe(
            &mut read_handle,
            &mut write_handle,
            &mut attrs,
            0,
        ))?;
    }

    Ok((
        ReadPipe {
            handle: read_handle,
        },
        WritePipe {
            handle: write_handle,
        },
    ))
}

impl ReadPipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            handle: open_file(path, GENERIC_READ, OPEN_EXISTING, false)?,
        })
    }

    pub fn null() -> Result<Self> {
        Self::open("nul")
    }
}

impl Drop for ReadPipe {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

unsafe impl Send for ReadPipe {}

impl Read for ReadPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes_read: DWORD = 0;
        unsafe {
            ok_nonzero(ReadFile(
                self.handle,
                mem::transmute(buf.as_mut_ptr()),
                buf.len() as DWORD,
                &mut bytes_read,
                ptr::null_mut(),
            ))
            .map_err(|e| e.into_io_error())?;
        }
        Ok(bytes_read as usize)
    }
}

impl WritePipe {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            handle: open_file(path, GENERIC_WRITE, CREATE_ALWAYS, false)?,
        })
    }

    pub fn null() -> Result<Self> {
        Ok(Self {
            handle: open_file("nul", GENERIC_WRITE, OPEN_EXISTING, false)?,
        })
    }
}

unsafe impl Send for WritePipe {}

impl Write for WritePipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes_written: DWORD = 0;
        unsafe {
            ok_nonzero(WriteFile(
                self.handle,
                mem::transmute(buf.as_ptr()),
                buf.len() as DWORD,
                &mut bytes_written,
                ptr::null_mut(),
            ))
            .map_err(|e| e.into_io_error())?;
        }
        Ok(bytes_written as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        unsafe {
            ok_nonzero(FlushFileBuffers(self.handle)).map_err(|e| e.into_io_error())?;
        }
        Ok(())
    }
}

impl Drop for WritePipe {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

fn open_file<P: AsRef<Path>>(
    path: P,
    access: DWORD,
    creation_disposition: DWORD,
    exclusive: bool,
) -> Result<HANDLE> {
    let mut file = to_utf16(path.as_ref());
    let share_mode = if exclusive {
        0
    } else {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    };

    let handle = unsafe {
        CreateFileW(
            /*lpFileName=*/ file.as_mut_ptr(),
            /*dwDesiredAccess=*/ access,
            /*dwShareMode=*/ share_mode,
            /*lpSecurityAttributes=*/ ptr::null_mut(),
            /*dwCreationDisposition=*/ creation_disposition,
            /*dwFlagsAndAttributes=*/ FILE_ATTRIBUTE_NORMAL,
            /*hTemplateFile=*/ ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::last_os_error());
    }

    unsafe {
        match ok_nonzero(SetHandleInformation(
            handle,
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )) {
            Err(e) => {
                CloseHandle(handle);
                Err(e)
            }
            Ok(_) => Ok(handle),
        }
    }
}
