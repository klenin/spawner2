use crate::process::{
    self, ExitStatus, Group, GroupRestrictions, Process, ProcessInfo, Stdio, ResourceUsage,
};
use crate::{Error, Result};

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// An action that is performed when the process terminates.
pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

/// Describes the termination reason for a process.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    /// Process was terminated due to limit violation.
    LimitViolation(process::LimitViolation),
    /// Process was terminated by ['Runner'].
    ///
    /// [`Runner`]: struct.Runner.html
    ManuallyTerminated,
}

/// Summary information about process's execution.
#[derive(Clone, Debug)]
pub struct RunnerReport {
    pub resource_usage: ResourceUsage,
    pub exit_status: ExitStatus,
    pub termination_reason: Option<TerminationReason>,
}

enum Message {
    Terminate,
    Suspend,
    Resume,
    ResetTimeUsage,
}

/// Used to control a process in a [`RunnerThread`].
/// Note that multiple runners can exist at the same time, allowing simultaneous control
/// over given process.
///
/// [`RunnerThread`]: struct.RunnerThread.html
#[derive(Clone)]
pub struct Runner(Sender<Message>);

/// Monitors the resource usage of a process, providing [`RunnerReport`] at the end.
///
/// [`RunnerReport`]: struct.RunnerReport.html
pub struct RunnerThread {
    runner: Runner,
    handle: JoinHandle<Result<RunnerReport>>,
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

impl RunnerThread {
    /// Spawns runner thread, returning a handle to it.
    pub fn spawn(
        info: ProcessInfo,
        stdio: Stdio,
        restrictions: GroupRestrictions,
        monitor_interval: Duration,
        on_terminate: Option<Box<OnTerminate>>,
    ) -> Result<Self> {
        let (sender, receiver) = channel();
        thread::Builder::new()
            .spawn(move || {
                RunnerThread::entry(
                    info,
                    stdio,
                    restrictions,
                    monitor_interval,
                    on_terminate,
                    receiver,
                )
            })
            .map_err(|_| Error::from("Cannot spawn RunnerThread"))
            .map(|handle| RunnerThread {
                handle: handle,
                runner: Runner(sender),
            })
    }

    pub fn runner(&self) -> Runner {
        self.runner.clone()
    }

    /// Waits for the runner thread to finish.
    pub fn join(self) -> Result<RunnerReport> {
        self.handle
            .join()
            .unwrap_or(Err(Error::from("Runner thread panicked")))
    }

    fn entry(
        info: ProcessInfo,
        stdio: Stdio,
        restrictions: GroupRestrictions,
        monitor_interval: Duration,
        on_terminate: Option<Box<OnTerminate>>,
        receiver: Receiver<Message>,
    ) -> Result<RunnerReport> {
        let mut group = Group::new(restrictions)?;
        let mut process = group.spawn(info, stdio)?;
        let result =
            RunnerThread::monitor_process(&mut process, &mut group, monitor_interval, receiver);
        let _ = group.terminate();
        if let Some(mut handler) = on_terminate {
            handler.on_terminate();
        }
        result
    }

    fn monitor_process(
        process: &mut Process,
        group: &mut Group,
        monitor_interval: Duration,
        receiver: Receiver<Message>,
    ) -> Result<RunnerReport> {
        let mut term_reason = None;
        let mut exited = false;
        loop {
            if let Some(exit_status) = process.exit_status()? {
                exited = true;
                let res_usage = group.resource_usage()?;
                if res_usage.active_processes == 0 {
                    return Ok(RunnerReport {
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
