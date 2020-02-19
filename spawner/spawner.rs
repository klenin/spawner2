use crate::pipe::{ReadPipe, WritePipe};
use crate::process::{
    ExitStatus, Group, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers,
    ProcessInfo, Stdio,
};
use crate::supervisor::Supervisor;
use crate::{Error, Result};

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Describes the termination reason for a process.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TerminationReason {
    WallClockTimeLimitExceeded,
    IdleTimeLimitExceeded,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessLimitExceeded,
    ActiveProcessLimitExceeded,
    ActiveNetworkConnectionLimitExceeded,
    TerminatedByRunner,
}

#[derive(Copy, Clone, Debug)]
pub struct IdleTimeLimit {
    pub total_idle_time: Duration,
    pub cpu_load_threshold: f64,
}

/// The limits that are imposed on a process group.
#[derive(Copy, Clone, Debug)]
pub struct ResourceLimits {
    pub idle_time: Option<IdleTimeLimit>,
    /// The maximum allowed amount of time for a process group.
    pub wall_clock_time: Option<Duration>,
    /// The maximum allowed amount of user-mode execution time for a process group.
    pub total_user_time: Option<Duration>,
    /// The maximum allowed memory usage, in bytes.
    pub max_memory_usage: Option<u64>,
    /// The maximum allowed amount of bytes written by a process group.
    pub total_bytes_written: Option<u64>,
    /// The maximum allowed number of processes created.
    pub total_processes_created: Option<usize>,
    /// The maximum allowed number of active processes.
    pub active_processes: Option<usize>,
    /// The maximum allowed number of active network connections.
    pub active_network_connections: Option<usize>,
}

pub enum ProgramMessage {
    Terminate,
    Suspend,
    Resume,
    StopTimeAccounting,
    ResumeTimeAccounting,
    ResetTime,
}

/// Summary information about process's execution.
#[derive(Clone, Debug)]
pub struct Report {
    pub wall_clock_time: Duration,
    pub memory: Option<GroupMemory>,
    pub io: Option<GroupIo>,
    pub timers: Option<GroupTimers>,
    pub pid_counters: Option<GroupPidCounters>,
    pub network: Option<GroupNetwork>,
    pub exit_status: ExitStatus,
    pub termination_reason: Option<TerminationReason>,
}

pub type MessageChannel = (Sender<ProgramMessage>, Receiver<ProgramMessage>);

pub struct SpawnedProgram {
    info: ProcessInfo,
    group: Option<Group>,
    stdio: Option<Stdio>,
    resource_limits: Option<ResourceLimits>,
    monitor_interval: Duration,
    wait_for_children: bool,
    msg_channel: MessageChannel,
}

pub struct Runner {
    sender: Sender<ProgramMessage>,
    handle: JoinHandle<Result<Report>>,
}

pub struct Spawner(Vec<Runner>);

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            wall_clock_time: None,
            idle_time: None,
            total_user_time: None,
            max_memory_usage: None,
            total_bytes_written: None,
            total_processes_created: None,
            active_processes: None,
            active_network_connections: None,
        }
    }
}

impl SpawnedProgram {
    pub fn new(info: ProcessInfo) -> Self {
        Self {
            info,
            group: None,
            stdio: None,
            resource_limits: None,
            monitor_interval: Duration::from_millis(1),
            wait_for_children: false,
            msg_channel: channel(),
        }
    }

    pub fn group(&mut self, group: Group) -> &mut Self {
        self.group = Some(group);
        self
    }

    pub fn resource_limits(&mut self, resource_limits: ResourceLimits) -> &mut Self {
        self.resource_limits = Some(resource_limits);
        self
    }

    pub fn monitor_interval(&mut self, monitor_interval: Duration) -> &mut Self {
        self.monitor_interval = monitor_interval;
        self
    }

    pub fn stdio(&mut self, stdio: Stdio) -> &mut Self {
        self.stdio = Some(stdio);
        self
    }

    pub fn wait_for_children(&mut self, wait: bool) -> &mut Self {
        self.wait_for_children = wait;
        self
    }

    pub fn msg_channel(&mut self, channel: MessageChannel) -> &mut Self {
        self.msg_channel = channel;
        self
    }
}

impl Runner {
    pub fn sender(&self) -> &Sender<ProgramMessage> {
        &self.sender
    }
}

impl Spawner {
    pub fn spawn<I>(programs: I) -> Self
    where
        I: IntoIterator<Item = SpawnedProgram>,
    {
        Self(
            programs
                .into_iter()
                .map(|prog| Runner {
                    sender: prog.msg_channel.0.clone(),
                    handle: thread::spawn(|| {
                        Supervisor::start_monitoring(
                            prog.info,
                            match prog.stdio {
                                Some(stdio) => stdio,
                                None => Stdio {
                                    stdin: ReadPipe::null()?,
                                    stdout: WritePipe::null()?,
                                    stderr: WritePipe::null()?,
                                },
                            },
                            match prog.group {
                                Some(g) => g,
                                None => Group::new()?,
                            },
                            prog.resource_limits.unwrap_or_default(),
                            prog.monitor_interval,
                            Some(prog.msg_channel.1),
                            prog.wait_for_children,
                        )
                    }),
                })
                .collect(),
        )
    }

    pub fn runners(&self) -> &[Runner] {
        &self.0
    }

    pub fn wait(self) -> Vec<Result<Report>> {
        self.0
            .into_iter()
            .map(|runner| {
                runner
                    .handle
                    .join()
                    .unwrap_or_else(|_| Err(Error::from("Runner thread panicked")))
            })
            .collect()
    }
}
