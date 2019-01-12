use std::mem;
use winapi::shared::minwindef::{DWORD, FALSE};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use winapi::um::winnt::HANDLE;

pub struct ThreadIterator {
    process_id: DWORD,
    snapshot: HANDLE,
    entry: THREADENTRY32,
    initialized: bool,
    end_reached: bool,
}

impl ThreadIterator {
    pub fn new(process_id: DWORD) -> Self {
        Self {
            process_id: process_id,
            snapshot: INVALID_HANDLE_VALUE,
            entry: unsafe { mem::zeroed() },
            initialized: false,
            end_reached: false,
        }
    }

    fn init(&mut self) -> bool {
        if !self.initialized {
            self.entry.dwSize = mem::size_of_val(&self.entry) as DWORD;
            unsafe {
                self.snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                    INVALID_HANDLE_VALUE => return false,
                    x => x,
                };
                if Thread32First(self.snapshot, &mut self.entry) == FALSE {
                    return false;
                }
            }
            self.initialized = true;
        }
        true
    }
}

impl Drop for ThreadIterator {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                CloseHandle(self.snapshot);
            }
        }
    }
}

impl Iterator for ThreadIterator {
    type Item = DWORD;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.init() || self.end_reached {
            return None;
        }

        let mut result: Option<Self::Item> = None;
        while result.is_none() {
            if self.entry.th32OwnerProcessID == self.process_id {
                result = Some(self.entry.th32ThreadID);
            }
            if unsafe { Thread32Next(self.snapshot, &mut self.entry) } == FALSE {
                self.end_reached = true;
                break;
            }
        }
        result
    }
}
