use crate::cmd::{Command, Environment, RedirectFlags};
use crate::driver::Warnings;

use spawner::pipe::{ReadPipe, WritePipe};
use spawner::process::{Group, ProcessInfo};
use spawner::windows::pipe::{ReadPipeExt, WritePipeExt};
use spawner::windows::process::{GroupExt, ProcessInfoExt, UiRestrictions};
use spawner::Result;

use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::thread;

use winapi::um::ioapiset::CancelSynchronousIo;

pub struct ConsoleReader(thread::JoinHandle<()>);

impl ConsoleReader {
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self(thread::spawn(f))
    }

    pub fn interrupt(self) {
        unsafe {
            // This will make io::stdin().read_line(...) return Err(...).
            CancelSynchronousIo(self.0.as_raw_handle());
        }
        let _ = self.0.join();
    }
}

pub fn open_input_file(
    file: &Path,
    flags: RedirectFlags,
    _warnings: &Warnings,
) -> Result<ReadPipe> {
    if flags.exclusive {
        ReadPipe::lock(file)
    } else {
        ReadPipe::open(file)
    }
}

pub fn open_output_file(
    file: &Path,
    flags: RedirectFlags,
    _warnings: &Warnings,
) -> Result<WritePipe> {
    if flags.exclusive {
        WritePipe::lock(file)
    } else {
        WritePipe::open(file)
    }
}

pub fn init_os_specific_process_extensions(
    cmd: &Command,
    info: &mut ProcessInfo,
    group: &mut Group,
    _warnings: &Warnings,
) -> Result<()> {
    if cmd.show_window {
        info.show_window(true);
    }
    if cmd.env == Environment::UserDefault {
        info.env_user();
    }
    if cmd.secure {
        group.set_ui_restrictions(
            UiRestrictions::new()
                .limit_desktop()
                .limit_display_settings()
                .limit_exit_windows()
                .limit_global_atoms()
                .limit_handles()
                .limit_read_clipboard()
                .limit_write_clipboard()
                .limit_system_parameters(),
        )?;
    }
    Ok(())
}
