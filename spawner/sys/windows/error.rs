use winapi::shared::minwindef::DWORD;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::{
    FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use winapi::um::winnt::{LANG_ENGLISH, MAKELANGID, SUBLANG_ENGLISH_US};

use std::char::{decode_utf16, REPLACEMENT_CHARACTER};
use std::fmt::{self, Write};
use std::ptr;

#[derive(Debug)]
pub struct SysError(DWORD);

impl SysError {
    pub fn last() -> Self {
        unsafe { Self(GetLastError()) }
    }

    pub fn raw(&self) -> DWORD {
        self.0
    }
}

impl std::error::Error for SysError {}

impl fmt::Display for SysError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut buf = [0_u16; 256];
        let msg_len = unsafe {
            FormatMessageW(
                /*dwFlags=*/
                FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                /*lpSource=*/ ptr::null(),
                /*dwMessageId=*/ self.0,
                /*dwLanguageId=*/ MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US) as DWORD,
                /*lpBuffer=*/ buf.as_mut_ptr(),
                /*nSize=*/ buf.len() as DWORD,
                /*Arguments=*/ ptr::null_mut(),
            ) as usize
        };

        if msg_len == 0 {
            f.write_str("Unable to format error message")
        } else {
            for c in decode_utf16(buf.iter().cloned().take(msg_len))
                .map(|r| r.unwrap_or(REPLACEMENT_CHARACTER))
            {
                f.write_char(c)?;
            }
            Ok(())
        }
    }
}
