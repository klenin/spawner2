use crate::process::{self, ExitStatus, Process, ResourceUsage};
use crate::task::OnTerminate;
use crate::{Error, Result};

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Describes the termination reason for a process.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    LimitViolation(process::LimitViolation),
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

    /// Terminates process.
    pub fn terminate(&self) {
        self.send(Message::Terminate)
    }

    /// Suspends process.
    pub fn suspend(&self) {
        self.send(Message::Suspend)
    }

    /// Resumes process.
    pub fn resume(&self) {
        self.send(Message::Resume)
    }

    /// Resets wall clock time, user time and idle time.
    pub fn reset_time_usage(&self) {
        self.send(Message::ResetTimeUsage)
    }
}

impl RunnerThread {
    /// Spawns runner thread.
    pub fn spawn(
        process: Process,
        resume_process: bool,
        monitor_interval: Duration,
        mut on_terminate: Option<Box<OnTerminate>>,
    ) -> Result<Self> {
        let (sender, receiver) = channel();
        thread::Builder::new()
            .spawn(move || {
                let result =
                    RunnerThread::entry(process, resume_process, monitor_interval, receiver);
                if let Some(handler) = on_terminate.as_mut() {
                    handler.on_terminate();
                }
                result
            })
            .map_err(|_| Error::from("Cannot spawn RunnerThread"))
            .map(|handle| RunnerThread {
                handle: handle,
                runner: Runner(sender),
            })
    }

    /// Returns runner.
    pub fn runner(&self) -> Runner {
        self.runner.clone()
    }

    /// Joins runner thread.
    pub fn join(self) -> Result<RunnerReport> {
        self.handle
            .join()
            .unwrap_or(Err(Error::from("Runner thread panicked")))
    }

    fn entry(
        mut process: Process,
        resume_process: bool,
        monitor_interval: Duration,
        receiver: Receiver<Message>,
    ) -> Result<RunnerReport> {
        if resume_process {
            process.resume()?;
        }

        let mut term_reason = None;
        loop {
            if let Some(exit_status) = process.exit_status()? {
                return Ok(RunnerReport {
                    resource_usage: process.resource_usage()?,
                    exit_status: exit_status,
                    termination_reason: term_reason,
                });
            }

            if let Some(lv) = process.check_limits()? {
                process.terminate()?;
                term_reason = Some(TerminationReason::LimitViolation(lv));
            }

            if let Ok(msg) = receiver.try_recv() {
                match msg {
                    Message::Terminate => {
                        process.terminate()?;
                        term_reason = Some(TerminationReason::ManuallyTerminated);
                    }
                    Message::Suspend => process.suspend()?,
                    Message::Resume => process.resume()?,
                    Message::ResetTimeUsage => process.reset_time_usage()?,
                }
            }

            thread::sleep(monitor_interval);
        }
    }
}
