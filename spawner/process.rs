use crate::pipe::{ReadPipe, WritePipe};
use crate::sys::process as imp;
use crate::sys::{AsInnerMut, IntoInner};
use crate::Result;

use std::time::Duration;

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

#[derive(Copy, Clone, Debug)]
pub struct GroupMemory {
    pub max_usage: u64,
}

#[derive(Copy, Clone, Debug)]
pub struct GroupTimers {
    pub total_user_time: Duration,
    pub total_kernel_time: Duration,
}

#[derive(Copy, Clone, Debug)]
pub struct GroupIo {
    pub total_bytes_written: u64,
}

#[derive(Copy, Clone, Debug)]
pub struct GroupPidCounters {
    pub active_processes: usize,
    pub total_processes: usize,
}

#[derive(Copy, Clone, Debug)]
pub struct GroupNetwork {
    pub active_connections: usize,
}

#[derive(Copy, Clone, Debug)]
pub enum OsLimit {
    Memory,
    ActiveProcess,
}

pub struct ResourceUsage<'a>(imp::ResourceUsage<'a>);

/// Describes a group of processes.
pub struct Group(imp::Group);

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

    pub fn spawn<T, U>(mut info: T, stdio: U) -> Result<Self>
    where
        T: AsMut<ProcessInfo>,
        U: Into<Stdio>,
    {
        imp::Process::spawn(info.as_mut().as_inner_mut(), stdio.into().into_inner()).map(Self)
    }

    pub fn spawn_in_group<T, U>(mut info: T, stdio: U, group: &mut Group) -> Result<Self>
    where
        T: AsMut<ProcessInfo>,
        U: Into<Stdio>,
    {
        imp::Process::spawn_in_group(
            &mut info.as_mut().0,
            stdio.into().into_inner(),
            &mut group.0,
        )
        .map(Self)
    }
}

impl<'a> ResourceUsage<'a> {
    pub fn new(group: &'a Group) -> Self {
        Self(imp::ResourceUsage::new(&group.0))
    }

    pub fn update(&mut self) -> Result<()> {
        self.0.update()
    }

    pub fn timers(&self) -> Result<Option<GroupTimers>> {
        self.0.timers()
    }

    pub fn memory(&self) -> Result<Option<GroupMemory>> {
        self.0.memory()
    }

    pub fn io(&self) -> Result<Option<GroupIo>> {
        self.0.io()
    }

    pub fn pid_counters(&self) -> Result<Option<GroupPidCounters>> {
        self.0.pid_counters()
    }

    pub fn network(&self) -> Result<Option<GroupNetwork>> {
        self.0.network()
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        imp::Group::new().map(Self)
    }

    pub fn add(&mut self, ps: &Process) -> Result<()> {
        self.0.add(&ps.0)
    }

    /// Returns `true` if the limit was set.
    pub fn set_os_limit(&mut self, limit: OsLimit, value: u64) -> Result<bool> {
        self.0.set_os_limit(limit, value)
    }

    /// Returns `true` if the limit was hit.
    pub fn is_os_limit_hit(&self, limit: OsLimit) -> Result<bool> {
        self.0.is_os_limit_hit(limit)
    }

    pub fn terminate(&self) -> Result<()> {
        self.0.terminate()
    }
}

impl IntoInner<imp::Stdio> for Stdio {
    fn into_inner(self) -> imp::Stdio {
        imp::Stdio {
            stdin: self.stdin.into_inner(),
            stdout: self.stdout.into_inner(),
            stderr: self.stderr.into_inner(),
        }
    }
}

impl AsInnerMut<imp::ProcessInfo> for ProcessInfo {
    fn as_inner_mut(&mut self) -> &mut imp::ProcessInfo {
        &mut self.0
    }
}

impl AsInnerMut<imp::Group> for Group {
    fn as_inner_mut(&mut self) -> &mut imp::Group {
        &mut self.0
    }
}

impl AsMut<ProcessInfo> for ProcessInfo {
    fn as_mut(&mut self) -> &mut ProcessInfo {
        self
    }
}

impl Default for GroupIo {
    fn default() -> Self {
        Self {
            total_bytes_written: 0,
        }
    }
}

impl Default for GroupMemory {
    fn default() -> Self {
        Self { max_usage: 0 }
    }
}

impl Default for GroupNetwork {
    fn default() -> Self {
        Self {
            active_connections: 0,
        }
    }
}

impl Default for GroupPidCounters {
    fn default() -> Self {
        Self {
            active_processes: 0,
            total_processes: 0,
        }
    }
}

impl Default for GroupTimers {
    fn default() -> Self {
        Self {
            total_user_time: Duration::from_millis(0),
            total_kernel_time: Duration::from_millis(0),
        }
    }
}
