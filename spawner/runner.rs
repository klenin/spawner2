use crate::process::{self, ExitStatus, Group, Process, ProcessStdio, ResourceUsage};
use crate::task::Task;
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
    pub fn spawn<T, U>(task: T, stdio: U) -> Result<Self>
    where
        T: Into<Task>,
        U: Into<ProcessStdio>,
    {
        let task = task.into();
        let stdio = stdio.into();
        let (sender, receiver) = channel();
        thread::Builder::new()
            .spawn(move || RunnerThread::entry(task, stdio, receiver))
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

    fn entry(task: Task, stdio: ProcessStdio, receiver: Receiver<Message>) -> Result<RunnerReport> {
        let mut group = Group::new()?;
        group.set_limits(task.resource_limits)?;
        let mut process = group.spawn(task.process_info, stdio)?;
        let result = RunnerThread::monitor_process(
            &mut process,
            &mut group,
            task.monitor_interval,
            receiver,
        );
        let _ = group.terminate();
        if let Some(mut handler) = task.on_terminate {
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
                        termination_reason: term_reason,
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
