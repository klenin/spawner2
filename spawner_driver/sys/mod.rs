#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
use crate::sys::windows as imp;

#[cfg(unix)]
use crate::sys::unix as imp;

use crate::cmd::{Command, RedirectFlags};
use crate::driver::Warnings;

use spawner::pipe::{ReadPipe, WritePipe};
use spawner::process::{Group, ProcessInfo};
use spawner::{Result, Run};

use std::io::{self, Write};
use std::path::Path;
use std::thread;
use std::time;

pub struct ConsoleReader(imp::ConsoleReader);

impl ConsoleReader {
    pub fn spawn(mut dst: WritePipe) -> Self {
        Self(imp::ConsoleReader::spawn(move || {
            let mut s = String::new();
            loop {
                s.clear();
                if io::stdin().read_line(&mut s).is_err() {
                    return;
                }
                if s.is_empty() || dst.write_all(s.as_bytes()).is_err() {
                    return;
                }
            }
        }))
    }

    pub fn join(self, run: &Run) {
        while !run.all_finished() {
            thread::sleep(time::Duration::from_millis(5));
        }

        // If spawner is launched from console then reader thread can wait for data from stdin forever.
        // Interrupt it in os-specific way.
        self.0.interrupt();
    }
}

pub fn open_input_file(file: &Path, flags: RedirectFlags, warnings: &Warnings) -> Result<ReadPipe> {
    imp::open_input_file(file, flags, warnings)
}

pub fn open_output_file(
    file: &Path,
    flags: RedirectFlags,
    warnings: &Warnings,
) -> Result<WritePipe> {
    imp::open_output_file(file, flags, warnings)
}

pub fn init_os_specific_process_extensions(
    cmd: &Command,
    info: &mut ProcessInfo,
    group: &mut Group,
    warnings: &Warnings,
) -> Result<()> {
    imp::init_os_specific_process_extensions(cmd, info, group, warnings)
}
