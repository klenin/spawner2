use crate::sys::windows::common::Handle;

use winapi::shared::minwindef::{DWORD, FALSE};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};

use std::mem;

struct ThreadInfo {
    snapshot: Handle,
    entry: THREADENTRY32,
}

pub struct ThreadIterator {
    process_id: DWORD,
    end_reached: bool,
    info: Option<ThreadInfo>,
}

impl ThreadIterator {
    pub fn new(process_id: DWORD) -> Self {
        Self {
            process_id: process_id,
            end_reached: false,
            info: None,
        }
    }
}

impl Iterator for ThreadIterator {
    type Item = DWORD;

    fn next(&mut self) -> Option<Self::Item> {
        if self.info.is_none() {
            self.info = ThreadInfo::create();
        }

        if self.info.is_none() || self.end_reached {
            return None;
        }

        let info = self.info.as_mut().unwrap();
        let mut result: Option<Self::Item> = None;
        while result.is_none() {
            if info.entry.th32OwnerProcessID == self.process_id {
                result = Some(info.entry.th32ThreadID);
            }
            if unsafe { Thread32Next(info.snapshot.0, &mut info.entry) } == FALSE {
                self.end_reached = true;
                break;
            }
        }
        result
    }
}

impl ThreadInfo {
    fn create() -> Option<Self> {
        unsafe {
            let mut entry: THREADENTRY32 = mem::zeroed();
            entry.dwSize = mem::size_of_val(&entry) as DWORD;
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                INVALID_HANDLE_VALUE => return None,
                x => Handle(x),
            };
            if Thread32First(snapshot.0, &mut entry) == FALSE {
                return None;
            }
            Some(ThreadInfo {
                entry: entry,
                snapshot: snapshot,
            })
        }
    }
}
