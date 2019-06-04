use crate::io::{IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId, StdioMapping};
use crate::pipe::{ReadPipe, WritePipe};
use crate::process::{
    self, ExitStatus, Group, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers,
    ProcessInfo,
};
use crate::runner::{RunnerData, RunnerMessage, RunnerThread};
use crate::rwhub::{ReadHub, ReaderThread, WriteHub};
use crate::{Error, Result};

use std::fmt;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

/// An action that is performed when the process terminates.
pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

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

#[derive(Clone)]
pub struct Runner(Sender<RunnerMessage>);

#[derive(Debug)]
pub struct SpawnerErrors(Vec<Error>);

pub type SpawnerResult = std::result::Result<Report, SpawnerErrors>;

pub struct SpawnedProgram {
    info: ProcessInfo,
    group: Option<Group>,
    stdio: Option<StdioMapping>,
    resource_limits: Option<ResourceLimits>,
    monitor_interval: Duration,
    on_terminate: Option<Box<OnTerminate>>,
    wait_for_children: bool,
}

struct Stdio {
    stdin_r: ReadPipe,
    stdin_w: Option<WriteHub>,
    stdout_r: Option<ReadHub>,
    stdout_w: WritePipe,
    stderr_r: Option<ReadHub>,
    stderr_w: WritePipe,
}

struct Program {
    runner: Runner,
    monitoring_thread: RunnerThread,
    stdout_reader: Option<ReaderThread>,
    stderr_reader: Option<ReaderThread>,
}

pub struct Spawner {
    programs: Vec<Program>,
    other_readers: Vec<ReaderThread>,
    other_writers: Vec<WriteHub>,
}

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

impl Runner {
    fn send(&self, m: RunnerMessage) {
        let _ = self.0.send(m);
    }

    pub fn terminate(&self) {
        self.send(RunnerMessage::Terminate)
    }

    pub fn suspend(&self) {
        self.send(RunnerMessage::Suspend)
    }

    pub fn resume(&self) {
        self.send(RunnerMessage::Resume)
    }

    pub fn reset_wallclock_and_user_time(&self) {
        self.send(RunnerMessage::ResetWallclockAndUserTime)
    }

    pub fn stop_time_accounting(&self) {
        self.send(RunnerMessage::StopTimeAccounting)
    }

    pub fn resume_time_accounting(&self) {
        self.send(RunnerMessage::ResumeTimeAccounting)
    }
}

impl SpawnerErrors {
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = &'a Error> + 'a {
        self.0.iter()
    }

    pub fn into_inner(self) -> Vec<Error> {
        self.0
    }
}

impl std::error::Error for SpawnerErrors {}

impl fmt::Display for SpawnerErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.iter() {
            writeln!(f, "{}", e)?;
        }
        Ok(())
    }
}

impl SpawnedProgram {
    pub fn new(info: ProcessInfo) -> Self {
        Self {
            info: info,
            group: None,
            stdio: None,
            resource_limits: None,
            monitor_interval: Duration::from_millis(1),
            on_terminate: None,
            wait_for_children: false,
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

    pub fn on_terminate<T>(&mut self, on_terminate: T) -> &mut Self
    where
        T: OnTerminate + 'static,
    {
        self.on_terminate = Some(Box::new(on_terminate));
        self
    }

    pub fn stdio(&mut self, stdio: StdioMapping) -> &mut Self {
        self.stdio = Some(stdio);
        self
    }

    pub fn wait_for_children(&mut self, wait: bool) -> &mut Self {
        self.wait_for_children = wait;
        self
    }
}

impl Spawner {
    pub fn spawn<I>(programs: I, mut iostreams: IoStreams) -> Result<Self>
    where
        I: IntoIterator<Item = SpawnedProgram>,
    {
        let mut active_programs = Vec::new();
        for p in programs.into_iter() {
            match Program::spawn(p, &mut iostreams) {
                Ok(p) => active_programs.push(p),
                Err(e) => {
                    for p in active_programs.iter() {
                        p.runner.terminate();
                    }
                    return Err(e);
                }
            }
        }

        let other_readers = iostreams
            .take_remaining_istreams()
            .map(|(_, stream)| stream.dst.start_reading())
            .collect();
        let other_writers = iostreams
            .take_remaining_ostreams()
            .map(|(_, stream)| stream.src)
            .collect();

        Ok(Self {
            programs: active_programs,
            other_readers: other_readers,
            other_writers: other_writers,
        })
    }

    pub fn runners<'a>(&'a self) -> impl Iterator<Item = Runner> + 'a {
        self.programs.iter().map(|p| p.runner.clone())
    }

    pub fn wait(self) -> Vec<SpawnerResult> {
        let result = self.programs.into_iter().map(Program::wait).collect();
        for reader in self.other_readers.into_iter() {
            let _ = reader.join();
        }
        drop(self.other_writers);
        result
    }
}

impl Stdio {
    fn from_iostreams(iostreams: &mut IoStreams, mapping: StdioMapping) -> Result<Self> {
        let stdio = iostreams.take_stdio(mapping);
        let graph = iostreams.graph();
        let (stdin_r, stdin_w) = ostream_endings(graph, stdio.stdin, mapping.stdin)?;
        let (stdout_w, stdout_r) = istream_endings(graph, stdio.stdout, mapping.stdout)?;
        let (stderr_w, stderr_r) = istream_endings(graph, stdio.stderr, mapping.stderr)?;
        Ok(Self {
            stdin_r: stdin_r,
            stdin_w: stdin_w,
            stdout_r: stdout_r,
            stdout_w: stdout_w,
            stderr_r: stderr_r,
            stderr_w: stderr_w,
        })
    }
}

impl Program {
    fn spawn(p: SpawnedProgram, iostreams: &mut IoStreams) -> Result<Self> {
        let (sender, receiver) = channel();
        let stdio = match p.stdio {
            Some(stdio) => Stdio::from_iostreams(iostreams, stdio)?,
            None => Stdio {
                stdin_r: ReadPipe::null()?,
                stdin_w: None,
                stdout_r: None,
                stdout_w: WritePipe::null()?,
                stderr_r: None,
                stderr_w: WritePipe::null()?,
            },
        };
        drop(stdio.stdin_w);

        Ok(Self {
            runner: Runner(sender),
            monitoring_thread: RunnerThread::spawn(RunnerData {
                info: p.info,
                stdio: process::Stdio {
                    stdin: stdio.stdin_r,
                    stdout: stdio.stdout_w,
                    stderr: stdio.stderr_w,
                },
                group: match p.group {
                    Some(group) => group,
                    None => Group::new()?,
                },
                limits: p.resource_limits.unwrap_or_default(),
                monitor_interval: p.monitor_interval,
                on_terminate: p.on_terminate,
                receiver: receiver,
                wait_for_children: p.wait_for_children,
            }),
            stdout_reader: stdio.stdout_r.map(ReadHub::start_reading),
            stderr_reader: stdio.stderr_r.map(ReadHub::start_reading),
        })
    }

    fn wait(self) -> SpawnerResult {
        let mut errors = SpawnerErrors(
            std::iter::once(self.stdout_reader)
                .chain(Some(self.stderr_reader))
                .filter_map(|reader_opt| {
                    reader_opt.map(|reader| reader.join().err()).unwrap_or(None)
                })
                .collect(),
        );

        match self.monitoring_thread.join() {
            Ok(report) => {
                if errors.0.is_empty() {
                    Ok(report)
                } else {
                    Err(errors)
                }
            }
            Err(e) => {
                errors.0.push(e);
                Err(errors)
            }
        }
    }
}

fn ostream_endings(
    graph: &IoGraph,
    ostream: Ostream,
    id: OstreamId,
) -> Result<(ReadPipe, Option<WriteHub>)> {
    Ok(if graph.ostream_edges(id).is_empty() {
        (ReadPipe::null()?, None)
    } else {
        (ostream.dst.unwrap(), Some(ostream.src))
    })
}

fn istream_endings(
    graph: &IoGraph,
    istream: Istream,
    id: IstreamId,
) -> Result<(WritePipe, Option<ReadHub>)> {
    Ok(if graph.istream_edges(id).is_empty() {
        (WritePipe::null()?, None)
    } else {
        (istream.src.unwrap(), Some(istream.dst))
    })
}
