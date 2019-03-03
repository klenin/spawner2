use crate::Result;
use command::{Command, EnvKind};
use pipe::{ReadPipe, WritePipe};
use std::time::Duration;
use sys::process as ps_impl;
use sys::IntoInner;

/// This structure is used to represent and manage root process and all its descendants.
pub struct Process(ps_impl::Process);

/// Describes current status of a process, returned by [`status`] method.
/// 
/// [`status`]: struct.Process.html#method.status
#[derive(Copy, Clone, Debug)]
pub enum ProcessStatus {
    Running,
    Finished(u32),
    Crashed(u32, &'static str),
}

/// Information about a process, returned by [`info`] method.
/// 
/// [`info`]: struct.Process.html#method.info
#[derive(Copy, Clone, Debug)]
pub struct ProcessInfo {
    /// The amount of time elapsed since process creation.
    pub wall_clock_time: Duration,
    /// The total amount of user-mode execution time for all active processes,
    /// as well as all terminated processes.
    pub total_user_time: Duration,
    /// The total amount of kernel-mode execution time for all active processes,
    /// as well as all terminated processes.
    pub total_kernel_time: Duration,
    /// The peak memory usage of all active processes, in bytes.
    pub peak_memory_used: u64,
    /// The total number of processes created.
    pub total_processes_created: usize,
    /// Total bytes written by a process.
    pub total_bytes_written: u64,
}

/// Contains standard input/output handles of a process.
pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

impl Process {
    /// Spawns process from a given command and stdio streams.
    pub fn spawn(cmd: &Command, stdio: ProcessStdio) -> Result<Self> {
        let mut builder = ps_impl::ProcessBuilder::new(
            std::iter::once(cmd.app.as_str()).chain(cmd.args.iter().map(|a| a.as_str())),
            ps_impl::ProcessStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        );
        builder
            .current_dir(cmd.current_dir.as_ref())
            .spawn_suspended(cmd.spawn_suspended)
            .show_window(cmd.show_gui);

        match cmd.env_kind {
            EnvKind::Clear => {
                builder.clear_env();
            }
            EnvKind::Inherit => {
                builder.inherit_env();
            }
            EnvKind::UserDefault => {
                builder.user_env()?;
            }
        }

        for var in cmd.env_vars.iter() {
            builder.env_var(&var.name, &var.val);
        }

        Ok(Self(builder.spawn()?))
    }

    /// Returns status of the root process. Note that [`ProcessStatus::Finished`] does not guarantee
    /// that all child processes are finished.
    ///
    /// [`Status::Finished`]: enum.ProcessStatus.html#variant.Finished
    pub fn status(&self) -> Result<ProcessStatus> {
        Ok(match self.0.exit_status()? {
            Some(status) => match status.crash_cause() {
                Some(cause) => ProcessStatus::Crashed(status.code(), cause),
                None => ProcessStatus::Finished(status.code()),
            },
            None => ProcessStatus::Running,
        })
    }

    /// Returns information about the root process and all its descendants.
    pub fn info(&self) -> Result<ProcessInfo> {
        let info = self.0.info()?;
        Ok(ProcessInfo {
            wall_clock_time: info.wall_clock_time(),
            total_user_time: info.total_user_time(),
            total_kernel_time: info.total_kernel_time(),
            peak_memory_used: info.peak_memory_used(),
            total_processes_created: info.total_processes_created(),
            total_bytes_written: info.total_bytes_written(),
        })
    }

    /// Suspends the root process.
    pub fn suspend(&self) -> Result<()> {
        self.0.suspend()
    }

    /// Resumes the root process.
    pub fn resume(&self) -> Result<()> {
        self.0.resume()
    }
}

impl ProcessInfo {
    pub fn zeroed() -> Self {
        Self {
            wall_clock_time: Duration::from_nanos(0),
            total_user_time: Duration::from_nanos(0),
            total_kernel_time: Duration::from_nanos(0),
            peak_memory_used: 0,
            total_bytes_written: 0,
            total_processes_created: 0,
        }
    }
}
