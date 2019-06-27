use crate::cmd::{Command, Environment, RedirectFlags};
use crate::driver::Warnings;

use spawner::pipe::{ReadPipe, WritePipe};
use spawner::process::{Group, ProcessInfo};
use spawner::Result;

use std::path::Path;

#[cfg(windows)]
pub fn open_input_file(
    file: &Path,
    flags: RedirectFlags,
    _warnings: &Warnings,
) -> Result<ReadPipe> {
    use spawner::windows::pipe::ReadPipeExt;
    if flags.exclusive {
        ReadPipe::lock(file)
    } else {
        ReadPipe::open(file)
    }
}

#[cfg(unix)]
pub fn open_input_file(file: &Path, flags: RedirectFlags, warnings: &Warnings) -> Result<ReadPipe> {
    if flags.exclusive {
        warnings.emit("Exclusive redirect works on windows only");
    }
    ReadPipe::open(file)
}

#[cfg(windows)]
pub fn open_output_file(
    file: &Path,
    flags: RedirectFlags,
    _warnings: &Warnings,
) -> Result<WritePipe> {
    use spawner::windows::pipe::WritePipeExt;
    if flags.exclusive {
        WritePipe::lock(file)
    } else {
        WritePipe::open(file)
    }
}

#[cfg(unix)]
pub fn open_output_file(
    file: &Path,
    flags: RedirectFlags,
    warnings: &Warnings,
) -> Result<WritePipe> {
    if flags.exclusive {
        warnings.emit("Exclusive redirect works on windows only");
    }
    WritePipe::open(file)
}

#[cfg(windows)]
pub fn init_os_specific_process_extensions(
    cmd: &Command,
    info: &mut ProcessInfo,
    group: &mut Group,
    _warnings: &Warnings,
) -> Result<()> {
    use spawner::windows::process::{GroupExt, ProcessInfoExt, UiRestrictions};

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

#[cfg(unix)]
pub fn init_os_specific_process_extensions(
    cmd: &Command,
    info: &mut ProcessInfo,
    group: &mut Group,
    warnings: &Warnings,
) -> Result<()> {
    use spawner::unix::process::{ProcessInfoExt, SyscallFilterBuilder};

    if cmd.show_window {
        warnings.emit("'-sw' option works on windows only");
    }
    if cmd.env == Environment::UserDefault {
        warnings.emit(
            "'-env=user-default' option works on windows only, '-env=inherit' will be used instead",
        );
        info.env_inherit();
    }

    // Syscall codes to allow execve.
    #[cfg(target_arch = "x86")]
    let syscall_codes = [
        173, // rt_sigreturn
        252, // exit_group
        1,   // exit
        3,   // read
        4,   // write
        175, // rt_sigprocmask
        174, // rt_sigaction
        162, // nanosleep
        45,  // brk
        11,  // execve
        6,   // close
        5,   // open
        33,  // access
        108, // fstat
        90,  // mmap
        91,  // munmap
        125, // mprotect
    ];

    #[cfg(target_arch = "x86_64")]
    let syscall_codes = [
        15,  // rt_sigreturn
        231, // exit_group
        60,  // exit
        0,   // read
        1,   // write
        14,  // rt_sigprocmask
        13,  // rt_sigaction
        35,  // nanosleep
        12,  // brk
        59,  // execve
        3,   // close
        2,   // open
        21,  // access
        5,   // fstat
        9,   // mmap
        158, // arch_prctl
        11,  // munmap
        10,  // mprotect
    ];

    if cmd.secure {
        let mut builder = SyscallFilterBuilder::block_all();
        for syscall in syscall_codes.iter() {
            builder.allow(*syscall);
        }
        info.syscall_filter(builder.build());
    }
    Ok(())
}
