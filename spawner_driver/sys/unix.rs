use crate::cmd::{Command, Environment, RedirectFlags};
use crate::driver::Warnings;

use spawner::pipe::{ReadPipe, WritePipe};
use spawner::process::{Group, ProcessInfo};
use spawner::Result;

use std::path::Path;

pub struct ConsoleReader(libc::pid_t);

impl ConsoleReader {
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        match unsafe { libc::fork() } {
            -1 => unreachable!("Cannot create ConsoleReader"),
            0 => {
                f();
                std::process::exit(0);
            }
            x => Self(x),
        }
    }

    pub fn interrupt(self) {
        // There's no way to interrupt reading thread. Just kill it.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}

pub fn open_input_file(file: &Path, flags: RedirectFlags, warnings: &Warnings) -> Result<ReadPipe> {
    if flags.exclusive {
        warnings.emit("Exclusive redirect works on windows only");
    }
    ReadPipe::open(file)
}

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

pub fn init_os_specific_process_extensions(
    cmd: &Command,
    info: &mut ProcessInfo,
    _group: &mut Group,
    warnings: &Warnings,
) -> Result<()> {
    use spawner::unix::process::{CpuSet, ProcessInfoExt, SyscallFilterBuilder};

    if cmd.show_window {
        warnings.emit("'-sw' option works on windows only");
    }
    if cmd.env == Environment::UserDefault {
        warnings.emit(
            "'-env=user-default' option works on windows only, '-env=inherit' will be used instead",
        );
        info.env_inherit();
    }

    // On unix C++ spawner runs all processes on the first core.
    let mut cpuset = CpuSet::new();
    cpuset.set(0)?;
    info.cpuset(cpuset);

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
