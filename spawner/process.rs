use crate::pipe::{ReadPipe, WritePipe};
use crate::sys::process as ps_impl;
use crate::sys::IntoInner;
use crate::Result;

use std::time::Duration;

/// Describes the limit that has been hit.
#[derive(Clone, Debug, PartialEq)]
pub enum LimitViolation {
    /// Process exceeded wall clock time limit.
    WallClockTimeLimitExceeded,
    /// Process exceeded idle time limit.
    IdleTimeLimitExceeded,
    /// Process exceeded user time limit.
    UserTimeLimitExceeded,
    /// Process exceeded write limit.
    WriteLimitExceeded,
    /// Process exceeded memory limit.
    MemoryLimitExceeded,
    /// Process created too many child processes.
    ProcessLimitExceeded,
}

/// The limits that are imposed on a process.
#[derive(Copy, Clone, Debug)]
pub struct ResourceLimits {
    /// The maximum allowed amount of time for a task.
    pub max_wall_clock_time: Option<Duration>,
    /// Idle time is wall clock time - user time.
    pub max_idle_time: Option<Duration>,
    /// The maximum allowed amount of user-mode execution time for a task.
    pub max_user_time: Option<Duration>,
    /// The maximum allowed memory usage, in bytes.
    pub max_memory_usage: Option<u64>,
    /// The maximum allowed amount of bytes written by a task.
    pub max_output_size: Option<u64>,
    /// The maximum allowed number of processes created.
    pub max_processes: Option<usize>,
}

/// Describes the resource usage of a process.
#[derive(Copy, Clone, Debug)]
pub struct ResourceUsage {
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
    /// Total bytes written by all active and all terminated processes.
    pub total_bytes_written: u64,
}

/// Describes the result of a process after it has terminated.
#[derive(Clone, Debug, PartialEq)]
pub enum ExitStatus {
    /// Process has crashed.
    Crashed(String),
    /// Process has exited.
    Finished(u32),
}

/// Describes the standard I/O streams of a process.
pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

/// Defines the default environment for a process.
#[derive(Copy, Clone, Debug)]
pub enum Environment {
    /// Clears the default environment.
    Clear,
    /// Inherits current process's environment.
    Inherit,
    /// Inherits default environment from a user.
    UserDefault,
}

/// Represents the set of parameters to use to spawn a process.
pub struct ProcessInfo {
    pub app: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub show_window: bool,
    pub resource_limits: ResourceLimits,
    pub env: Environment,
    pub env_vars: Vec<(String, String)>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Handle to a process.
pub struct Process(ps_impl::Process);

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_wall_clock_time: None,
            max_idle_time: None,
            max_user_time: None,
            max_memory_usage: None,
            max_output_size: None,
            max_processes: None,
        }
    }
}

impl Default for ResourceUsage {
    fn default() -> Self {
        Self {
            wall_clock_time: Duration::from_millis(0),
            total_user_time: Duration::from_millis(0),
            total_kernel_time: Duration::from_millis(0),
            peak_memory_used: 0,
            total_processes_created: 0,
            total_bytes_written: 0,
        }
    }
}

impl Process {
    /// Spawns process in suspended state.
    pub fn suspended<T, U>(info: T, stdio: U) -> Result<Self>
    where
        T: AsRef<ProcessInfo>,
        U: Into<ProcessStdio>,
    {
        let stdio = stdio.into();
        ps_impl::Process::suspended(
            info,
            ps_impl::ProcessStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )
        .map(Self)
    }

    /// Returns exit status of the root process if the process tree has terminated.
    /// Returns `Ok(None)` if at least one child process is alive.
    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        self.0.exit_status()
    }

    /// Suspends the main thread of the root process.
    pub fn suspend(&self) -> Result<()> {
        self.0.suspend()
    }

    /// Resumes the main thread of the root process.
    pub fn resume(&self) -> Result<()> {
        self.0.resume()
    }

    /// Resets wall clock time, user time and idle time usage.
    /// Does not change values in the current resource usage of a process.
    pub fn reset_time_usage(&mut self) -> Result<()> {
        self.0.reset_time_usage()
    }

    // Checks process for limit violation.
    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        self.0.check_limits()
    }

    /// Returns current resource usage.
    pub fn resource_usage(&mut self) -> Result<ResourceUsage> {
        self.0.resource_usage()
    }

    /// Terminates process along with all its children.
    /// If process tree has already been terminated, an `Ok` is returned.
    pub fn terminate(&self) -> Result<()> {
        self.0.terminate()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

impl AsRef<ProcessInfo> for ProcessInfo {
    fn as_ref(&self) -> &ProcessInfo {
        self
    }
}
