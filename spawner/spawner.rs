use crate::io::{IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId, StdioMapping};
use crate::pipe::{ReadPipe, WritePipe};
use crate::process::{
    self, ExitStatus, Group, GroupRestrictions, Process, ProcessInfo, ResourceUsage,
};
use crate::rwhub::{Connection, ReadHub, WriteHub};
use crate::{Error, Result};

use std::fmt;
use std::io::Read;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// An action that is performed when the process terminates.
pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

/// An action that is performed when [`Spawner`] reads from stdout or stderr.
///
/// [`Spawner`]: struct.Spawner.html
pub trait OnRead: Send {
    fn on_read(&mut self, data: &[u8], connections: &mut [Connection]) -> Result<()>;
}

/// Describes the termination reason for a process.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    LimitViolation(process::LimitViolation),
    /// Process was terminated by [`Runner`].
    ///
    /// [`Runner`]: struct.Runner.html
    ManuallyTerminated,
}

/// Summary information about process's execution.
#[derive(Clone, Debug)]
pub struct Report {
    pub resource_usage: ResourceUsage,
    pub exit_status: ExitStatus,
    pub termination_reason: Option<TerminationReason>,
}

/// Describes the set of actions performed by [`Spawner`].
///
/// [`Spawner`]: struct.Spawner.html
pub struct Actions {
    on_terminate: Option<Box<OnTerminate>>,
    on_stdout_read: Option<Box<OnRead>>,
    on_stderr_read: Option<Box<OnRead>>,
}

struct ReaderThread(JoinHandle<Result<ReadPipe>>);

pub struct Router {
    readers: Vec<ReaderThread>,
    _writers: Vec<WriteHub>,
}

struct RunnerThread(JoinHandle<Result<Report>>);

enum Message {
    Terminate,
    Suspend,
    Resume,
    ResetTimeUsage,
}

#[derive(Clone)]
pub struct Runner(Sender<Message>);

#[derive(Debug)]
pub struct SpawnerErrors(Vec<Error>);

pub type SpawnerResult = std::result::Result<Report, SpawnerErrors>;

pub struct Spawner {
    runner: Runner,
    monitoring_thread: RunnerThread,
    stdout_reader: Option<ReaderThread>,
    stderr_reader: Option<ReaderThread>,
}

pub struct Builder {
    info: ProcessInfo,
    restrictions: GroupRestrictions,
    monitor_interval: Duration,
    actions: Actions,
}

impl Actions {
    pub fn new() -> Self {
        Self {
            on_terminate: None,
            on_stdout_read: None,
            on_stderr_read: None,
        }
    }

    pub fn on_terminate<T>(mut self, on_terminate: T) -> Self
    where
        T: OnTerminate + 'static,
    {
        self.on_terminate = Some(Box::new(on_terminate));
        self
    }

    pub fn on_stdout_read<T>(mut self, on_read: T) -> Self
    where
        T: OnRead + 'static,
    {
        self.on_stdout_read = Some(Box::new(on_read));
        self
    }

    pub fn on_stderr_read<T>(mut self, on_read: T) -> Self
    where
        T: OnRead + 'static,
    {
        self.on_stderr_read = Some(Box::new(on_read));
        self
    }
}

impl ReaderThread {
    fn spawn(mut rh: ReadHub, mut on_read: Option<Box<OnRead>>) -> Self {
        Self(thread::spawn(move || {
            let mut buffer: Vec<u8> = Vec::new();
            buffer.resize(8192, 0);

            loop {
                let bytes_read = match rh.read(buffer.as_mut_slice()) {
                    Ok(x) => x,
                    Err(_) => break,
                };
                if bytes_read == 0 {
                    break;
                }

                let data = &buffer[..bytes_read];
                match on_read {
                    Some(ref mut handler) => handler.on_read(data, rh.connections_mut())?,
                    None => rh.transmit(data),
                }

                if rh.connections().iter().all(Connection::is_dead) {
                    break;
                }
            }
            Ok(rh.into_inner())
        }))
    }

    fn join(self) -> Result<ReadPipe> {
        self.0
            .join()
            .unwrap_or(Err(Error::from("ReaderThread panicked")))
    }
}

impl Router {
    pub fn from_iostreams(iostreams: &mut IoStreams) -> Self {
        let readers = iostreams
            .take_remaining_istreams()
            .map(|(_, stream)| ReaderThread::spawn(stream.dst, None))
            .collect();
        let writers = iostreams
            .take_remaining_ostreams()
            .map(|(_, stream)| stream.src)
            .collect();
        Self {
            readers: readers,
            _writers: writers,
        }
    }

    pub fn wait(self) {
        for reader in self.readers.into_iter() {
            let _ = reader.join();
        }
    }
}

impl RunnerThread {
    fn spawn(
        info: ProcessInfo,
        stdio: process::Stdio,
        restrictions: GroupRestrictions,
        monitor_interval: Duration,
        on_terminate: Option<Box<OnTerminate>>,
        receiver: Receiver<Message>,
    ) -> Self {
        Self(thread::spawn(move || {
            let mut group = Group::with_restrictions(restrictions)?;
            let mut process = group.spawn(info, stdio)?;
            let result =
                RunnerThread::monitor_process(&mut process, &mut group, monitor_interval, receiver);
            let _ = group.terminate();
            if let Some(mut handler) = on_terminate {
                handler.on_terminate();
            }
            result
        }))
    }

    fn join(self) -> Result<Report> {
        self.0
            .join()
            .unwrap_or(Err(Error::from("RunnerThread panicked")))
    }

    fn monitor_process(
        process: &mut Process,
        group: &mut Group,
        monitor_interval: Duration,
        receiver: Receiver<Message>,
    ) -> Result<Report> {
        let mut term_reason = None;
        let mut exited = false;
        loop {
            if let Some(exit_status) = process.exit_status()? {
                exited = true;
                let res_usage = group.resource_usage()?;
                if res_usage.active_processes == 0 {
                    return Ok(Report {
                        resource_usage: res_usage,
                        exit_status: exit_status,
                        termination_reason: term_reason.or_else(|| {
                            group
                                .check_limits()
                                .unwrap_or(None)
                                .map(TerminationReason::LimitViolation)
                        }),
                    });
                }
            }

            if let Some(lv) = group.check_limits()? {
                group.terminate()?;
                term_reason = Some(TerminationReason::LimitViolation(lv));
            }

            if let Ok(msg) = receiver.try_recv() {
                match msg {
                    Message::Terminate => {
                        group.terminate()?;
                        term_reason = Some(TerminationReason::ManuallyTerminated);
                    }
                    Message::Suspend => {
                        if !exited {
                            process.suspend()?;
                        }
                    }
                    Message::Resume => {
                        if !exited {
                            process.resume()?;
                        }
                    }
                    Message::ResetTimeUsage => group.reset_time_usage()?,
                }
            }

            thread::sleep(monitor_interval);
        }
    }
}

impl Runner {
    fn send(&self, m: Message) {
        let _ = self.0.send(m);
    }

    /// Sends message to a runner thread that will terminate process group.
    /// This method will do nothing if a runner thread has exited.
    pub fn terminate(&self) {
        self.send(Message::Terminate)
    }

    /// Sends message to a runner thread that will suspend the main thread of a running process.
    /// This method will do nothing if a runner thread has exited.
    pub fn suspend(&self) {
        self.send(Message::Suspend)
    }

    /// Sends message to a runner thread that will resume the main thread of a running process.
    /// This method will do nothing if a runner thread has exited.
    pub fn resume(&self) {
        self.send(Message::Resume)
    }

    /// Sends message to a runner thread that will reset wall clock,
    /// user and idle time usage of a process group.
    /// This method will do nothing if a runner thread has exited.
    pub fn reset_time_usage(&self) {
        self.send(Message::ResetTimeUsage)
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

impl Spawner {
    pub fn spawn(
        info: ProcessInfo,
        io_streams: &mut IoStreams,
        stdio_mapping: StdioMapping,
        restrictions: GroupRestrictions,
        monitor_interval: Duration,
        actions: Actions,
    ) -> Result<Self> {
        let stdio = io_streams.take_stdio(stdio_mapping);
        let graph = io_streams.graph();
        let (sender, receiver) = channel();

        let (stdin_r, _stdin_w) = ostream_endings(graph, stdio.stdin, stdio_mapping.stdin)?;
        let (stdout_w, stdout_r) = istream_endings(graph, stdio.stdout, stdio_mapping.stdout)?;
        let (stderr_w, stderr_r) = istream_endings(graph, stdio.stderr, stdio_mapping.stderr)?;

        let on_terminate = actions.on_terminate;
        let on_stdout_read = actions.on_stdout_read;
        let on_stderr_read = actions.on_stderr_read;

        Ok(Self {
            runner: Runner(sender),
            monitoring_thread: RunnerThread::spawn(
                info,
                process::Stdio {
                    stdin: stdin_r,
                    stdout: stdout_w,
                    stderr: stderr_w,
                },
                restrictions,
                monitor_interval,
                on_terminate,
                receiver,
            ),
            stdout_reader: stdout_r.map(|rh| ReaderThread::spawn(rh, on_stdout_read)),
            stderr_reader: stderr_r.map(|rh| ReaderThread::spawn(rh, on_stderr_read)),
        })
    }

    pub fn runner(&self) -> Runner {
        self.runner.clone()
    }

    pub fn wait(self) -> SpawnerResult {
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

impl Builder {
    pub fn new(info: ProcessInfo) -> Self {
        Self {
            info: info,
            restrictions: GroupRestrictions::new(),
            monitor_interval: Duration::from_millis(1),
            actions: Actions::new(),
        }
    }

    pub fn group_restrictions(&mut self, gr: GroupRestrictions) -> &mut Self {
        self.restrictions = gr;
        self
    }

    pub fn monitor_interval(&mut self, mi: Duration) -> &mut Self {
        self.monitor_interval = mi;
        self
    }

    pub fn actions(&mut self, actions: Actions) -> &mut Self {
        self.actions = actions;
        self
    }

    pub fn build(self, io_streams: &mut IoStreams, stdio_mapping: StdioMapping) -> Result<Spawner> {
        Spawner::spawn(
            self.info,
            io_streams,
            stdio_mapping,
            self.restrictions,
            self.monitor_interval,
            self.actions,
        )
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
