use crate::pipe::{ReadPipe, WritePipe};
use crate::sys::process as imp;
use crate::sys::{AsInnerMut, IntoInner};
use crate::Result;

use std::time::Duration;

/// Describes the limit that has been hit.
#[derive(Clone, Debug, PartialEq)]
pub enum LimitViolation {
    WallClockTimeLimitExceeded,
    IdleTimeLimitExceeded,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessLimitExceeded,
    ActiveProcessLimitExceeded,
    ActiveNetworkConnectionLimitExceeded,
}

/// The limits that are imposed on a process group.
#[derive(Copy, Clone, Debug)]
pub struct ResourceLimits {
    /// The maximum allowed amount of time for a process group.
    pub wall_clock_time: Option<Duration>,
    /// Idle time is wall clock time - user time.
    pub total_idle_time: Option<Duration>,
    /// The maximum allowed amount of user-mode execution time for a process group.
    pub total_user_time: Option<Duration>,
    /// The maximum allowed memory usage, in bytes.
    pub peak_memory_used: Option<u64>,
    /// The maximum allowed amount of bytes written by a process group.
    pub total_bytes_written: Option<u64>,
    /// The maximum allowed number of processes created.
    pub total_processes_created: Option<usize>,
    /// The maximum allowed number of active processes.
    pub active_processes: Option<usize>,
    /// The maximum allowed number of active network connections.
    pub active_network_connections: Option<usize>,
}

/// Describes the resource usage of a process group.
#[derive(Copy, Clone, Debug)]
pub struct ResourceUsage {
    /// The amount of time elapsed since process group creation.
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
    /// The number of active processes.
    pub active_processes: usize,
    /// Total bytes written by all active and all terminated processes.
    pub total_bytes_written: u64,
    /// The number of active network connections (both incoming and outgoing).
    pub active_network_connections: usize,
}

/// Describes the result of a process after it has terminated.
#[derive(Clone, Debug, PartialEq)]
pub enum ExitStatus {
    Crashed(String),
    Finished(u32),
}

/// Describes the standard I/O streams of a process.
pub struct Stdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

/// Represents the set of parameters to use to spawn a process.
pub struct ProcessInfo(imp::ProcessInfo);

/// Handle to a process.
pub struct Process(imp::Process);

pub struct GroupRestrictions(imp::GroupRestrictions);

/// Describes a group of processes.
pub struct Group(imp::Group);

impl ResourceLimits {
    pub fn new() -> Self {
        Self {
            wall_clock_time: None,
            total_idle_time: None,
            total_user_time: None,
            peak_memory_used: None,
            total_bytes_written: None,
            total_processes_created: None,
            active_processes: None,
            active_network_connections: None,
        }
    }
}

impl ResourceUsage {
    pub fn new() -> Self {
        Self {
            wall_clock_time: Duration::from_millis(0),
            total_user_time: Duration::from_millis(0),
            total_kernel_time: Duration::from_millis(0),
            peak_memory_used: 0,
            total_processes_created: 0,
            active_processes: 0,
            total_bytes_written: 0,
            active_network_connections: 0,
        }
    }
}

impl ProcessInfo {
    pub fn new<T: AsRef<str>>(app: T) -> Self {
        Self(imp::ProcessInfo::new(app))
    }

    pub fn args<T, U>(&mut self, args: T) -> &mut Self
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        self.0.args(args);
        self
    }

    pub fn envs<I, K, V>(&mut self, envs: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.0.envs(envs);
        self
    }

    pub fn working_dir<T: AsRef<str>>(&mut self, dir: T) -> &mut Self {
        self.0.working_dir(dir);
        self
    }

    pub fn suspended(&mut self, v: bool) -> &mut Self {
        self.0.suspended(v);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.0.env_clear();
        self
    }

    pub fn env_inherit(&mut self) -> &mut Self {
        self.0.env_inherit();
        self
    }

    pub fn user<T, U>(&mut self, username: T, password: Option<U>) -> &mut Self
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        self.0.user(username, password);
        self
    }
}

impl AsInnerMut<imp::ProcessInfo> for ProcessInfo {
    fn as_inner_mut(&mut self) -> &mut imp::ProcessInfo {
        &mut self.0
    }
}

impl Process {
    pub fn exit_status(&mut self) -> Result<Option<ExitStatus>> {
        self.0.exit_status()
    }

    /// Suspends the main thread of a process.
    pub fn suspend(&self) -> Result<()> {
        self.0.suspend()
    }

    /// Resumes the main thread of a process.
    pub fn resume(&self) -> Result<()> {
        self.0.resume()
    }

    pub fn terminate(&self) -> Result<()> {
        self.0.terminate()
    }
}

impl GroupRestrictions {
    pub fn new() -> Self {
        Self(imp::GroupRestrictions::new())
    }

    pub fn with_limits<T>(limits: T) -> Self
    where
        T: Into<ResourceLimits>,
    {
        Self(imp::GroupRestrictions::with_limits(limits))
    }
}

impl AsInnerMut<imp::GroupRestrictions> for GroupRestrictions {
    fn as_inner_mut(&mut self) -> &mut imp::GroupRestrictions {
        &mut self.0
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        imp::Group::new().map(Self)
    }

    pub fn with_restrictions<T>(restrictions: T) -> Result<Self>
    where
        T: Into<GroupRestrictions>,
    {
        imp::Group::with_restrictions(restrictions.into().0).map(Self)
    }

    pub fn spawn<T, U>(&mut self, mut info: T, stdio: U) -> Result<Process>
    where
        T: AsMut<ProcessInfo>,
        U: Into<Stdio>,
    {
        let stdio = stdio.into();
        self.0
            .spawn(
                &mut info.as_mut().0,
                imp::Stdio {
                    stdin: stdio.stdin.into_inner(),
                    stdout: stdio.stdout.into_inner(),
                    stderr: stdio.stderr.into_inner(),
                },
            )
            .map(Process)
    }

    pub fn resource_usage(&mut self) -> Result<ResourceUsage> {
        self.0.resource_usage()
    }

    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        self.0.check_limits()
    }

    /// Resets wall clock time, user time and idle time usage.
    /// Does not change values in the current resource usage.
    pub fn reset_time_usage(&mut self) -> Result<()> {
        self.0.reset_time_usage()
    }

    pub fn terminate(&self) -> Result<()> {
        self.0.terminate()
    }
}

impl AsMut<ProcessInfo> for ProcessInfo {
    fn as_mut(&mut self) -> &mut ProcessInfo {
        self
    }
}
