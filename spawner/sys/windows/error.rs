use winapi::shared::minwindef::DWORD;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::{
    FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use winapi::um::winnt::{LANG_ENGLISH, MAKELANGID, SUBLANG_ENGLISH_US, WCHAR};

use std::ptr;

pub fn last_os_error() -> String {
    let mut buf = [0 as WCHAR; 256];
    unsafe {
        let ecode = GetLastError();
        let msg_len = FormatMessageW(
            /*dwFlags=*/
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            /*lpSource=*/ ptr::null(),
            /*dwMessageId=*/ ecode,
            /*dwLanguageId=*/ MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US) as DWORD,
            /*lpBuffer=*/ buf.as_mut_ptr(),
            /*nSize=*/ buf.len() as DWORD,
            /*Arguments=*/ ptr::null_mut(),
        );
        if msg_len == 0 {
            String::from("Unable to format error message")
        } else {
            String::from_utf16_lossy(&buf[..msg_len as usize])
        }
    }
}
