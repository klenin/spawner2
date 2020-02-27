use crate::dataflow::{DestinationId, Graph, SourceId, Transmitter, TransmitterResults};
use crate::dataflow_analysis::DataflowOptimizer;
use crate::pipe;
use crate::process::{
    ExitStatus, Group, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers,
    ProcessInfo, Stdio,
};
use crate::supervisor::Supervisor;
use crate::{Error, Result};

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
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

#[derive(Debug)]
pub struct ProgramErrors {
    pub errors: Vec<Error>,
}

pub type ProgramResult = std::result::Result<Report, ProgramErrors>;

pub struct Program {
    info: ProcessInfo,
    group: Option<Group>,
    resource_limits: Option<ResourceLimits>,
    msg_receiver: Option<Receiver<ProgramMessage>>,
    monitor_interval: Duration,
    wait_for_children: bool,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: DestinationId,
    pub stdout: SourceId,
    pub stderr: SourceId,
}

struct ProgramExt {
    prog: Program,
    stdio: Stdio,
}

#[derive(Default)]
pub struct Session {
    progs: Vec<ProgramExt>,
    mappings: Vec<StdioMapping>,
    graph: Graph,
}

struct FlagGuard(Arc<AtomicBool>);

struct SupervisorThread {
    handle: JoinHandle<Result<Report>>,
    is_finished: Arc<AtomicBool>,
}

pub struct Run {
    supervisors: Vec<SupervisorThread>,
    mappings: Vec<StdioMapping>,
    transmitter: Transmitter,
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

impl std::error::Error for ProgramErrors {}

impl fmt::Display for ProgramErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.errors.iter() {
            writeln!(f, "{}", e)?;
        }
        Ok(())
    }
}

impl Program {
    pub fn new(info: ProcessInfo) -> Self {
        Self {
            info,
            group: None,
            resource_limits: None,
            // stdio: None,
            monitor_interval: Duration::from_millis(1),
            wait_for_children: false,
            msg_receiver: None,
        }
    }

    pub fn new_with<F>(info: ProcessInfo, f: F) -> Self
    where
        F: FnOnce(&mut Self),
    {
        let mut p = Self::new(info);
        f(&mut p);
        p
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

    pub fn wait_for_children(&mut self, wait: bool) -> &mut Self {
        self.wait_for_children = wait;
        self
    }

    pub fn msg_receiver(&mut self, receiver: Receiver<ProgramMessage>) -> &mut Self {
        self.msg_receiver = Some(receiver);
        self
    }
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_program<P>(&mut self, p: P) -> Result<StdioMapping>
    where
        P: Into<Program>,
    {
        let (stdin_r, stdin_w) = pipe::create()?;
        let (stdout_r, stdout_w) = pipe::create()?;
        let (stderr_r, stderr_w) = pipe::create()?;
        let mapping = StdioMapping {
            stdin: self.graph.add_destination(stdin_w),
            stdout: self.graph.add_source(stdout_r),
            stderr: self.graph.add_source(stderr_r),
        };
        self.progs.push(ProgramExt {
            prog: p.into(),
            stdio: Stdio {
                stdin: stdin_r,
                stdout: stdout_w,
                stderr: stderr_w,
            },
        });
        self.mappings.push(mapping);
        Ok(mapping)
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn run(mut self) -> Result<Run> {
        self.optimize_io().map(|_| Run {
            supervisors: self
                .progs
                .into_iter()
                .map(|p| SupervisorThread::spawn(p.prog, p.stdio))
                .collect(),
            transmitter: self.graph.transmit_data(),
            mappings: self.mappings,
        })
    }

    fn optimize_io(&mut self) -> Result<()> {
        let mut optimizer =
            DataflowOptimizer::new(&mut self.graph, &self.ignored_srcs, &self.ignored_dsts);
        for (mapping, prog) in self.mappings.iter().zip(self.progs.iter_mut()) {
            let stdio = &mut prog.stdio;
            optimizer.optimize_destination(mapping.stdin, &mut stdio.stdin)?;
            optimizer.optimize_source(mapping.stdout, &mut stdio.stdout)?;
            optimizer.optimize_source(mapping.stderr, &mut stdio.stderr)?;
        }
        Ok(())
    }
}

impl Drop for FlagGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl SupervisorThread {
    fn spawn(p: Program, stdio: Stdio) -> Self {
        let is_finished = Arc::new(AtomicBool::new(false));
        Self {
            is_finished: is_finished.clone(),
            handle: thread::spawn(|| {
                let _guard = FlagGuard(is_finished);
                Supervisor::start_monitoring(
                    p.info,
                    stdio,
                    match p.group {
                        Some(g) => g,
                        None => Group::new()?,
                    },
                    p.resource_limits.unwrap_or_default(),
                    p.monitor_interval,
                    p.msg_receiver,
                    p.wait_for_children,
                )
            }),
        }
    }

    fn wait(self, mapping: StdioMapping, io_errs: &mut Option<dataflow::Errors>) -> ProgramResult {
        // Collect io errors for this program.
        let mut errs = [mapping.stdout, mapping.stderr]
            .iter()
            .filter_map(|id| io_errs.as_mut().map(|e| e.errors.remove(id)).flatten())
            .collect::<Vec<_>>();

        let result = self
            .handle
            .join()
            .unwrap_or_else(|_| Err(Error::from("Supervisor thread panicked")))
            .map_err(|e| {
                errs.push(e);
            })
            .ok();
        if errs.is_empty() {
            Ok(result.unwrap())
        } else {
            Err(ProgramErrors { errors: errs })
        }
    }

    fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Acquire)
    }
}

impl Run {
    pub fn wait(self) -> Vec<ProgramResult> {
        let mut transmitter_errors = self.transmitter.wait().err();
        self.supervisors
            .into_iter()
            .zip(self.mappings.into_iter())
            .map(|(supervisor, mapping)| supervisor.wait(mapping, &mut transmitter_errors))
            .collect::<Vec<_>>()
    }

    pub fn all_finished(&self) -> bool {
        self.supervisors.iter().all(SupervisorThread::is_finished)
    }
}
