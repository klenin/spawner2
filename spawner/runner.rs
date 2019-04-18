use crate::pipe::{ReadPipe, WritePipe};
use crate::sys::runner as runner_impl;
use crate::sys::IntoInner;
use crate::task::{OnTerminate, ResourceLimits, Task};
use crate::{Error, Result};

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    WallClockTimeLimitExceeded,
    IdleTimeLimitExceeded,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessLimitExceeded,
    ManuallyTerminated,
}

#[derive(Copy, Clone, Debug)]
pub struct Statistics {
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
    /// Total bytes written by a process.
    pub total_bytes_written: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExitStatus {
    Crashed(String),
    Finished(u32),
    Terminated(TerminationReason),
}

#[derive(Clone, Debug)]
pub struct RunnerReport {
    pub statistics: Statistics,
    pub exit_status: ExitStatus,
}

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

pub struct Process(runner_impl::Process);

enum Message {
    Terminate,
    Suspend,
    Resume,
    ResetTimers,
}

pub struct Runner<'a>(runner_impl::Runner<'a>);

#[derive(Clone)]
pub struct RunnerController(Sender<Message>);

pub struct RunnerThread {
    ctl: RunnerController,
    handle: JoinHandle<Result<RunnerReport>>,
}

impl Statistics {
    pub fn zeroed() -> Self {
        Self {
            wall_clock_time: Duration::from_nanos(0),
            total_user_time: Duration::from_nanos(0),
            total_kernel_time: Duration::from_nanos(0),
            peak_memory_used: 0,
            total_bytes_written: 0,
            total_processes_created: 0,
        }
    }
}

impl Process {
    pub fn suspended(task: &Task, stdio: ProcessStdio) -> Result<Self> {
        runner_impl::Process::suspended(
            task,
            runner_impl::ProcessStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )
        .map(Self)
    }
}

impl<'a> Runner<'a> {
    pub fn new(ps: &'a mut Process, limits: ResourceLimits) -> Self {
        Self(runner_impl::Runner::new(&mut ps.0, limits))
    }

    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        self.0.exit_status()
    }

    pub fn suspend_process(&self) -> Result<()> {
        self.0.suspend_process()
    }

    pub fn resume_process(&self) -> Result<()> {
        self.0.resume_process()
    }

    pub fn reset_timers(&mut self) -> Result<()> {
        self.0.reset_timers()
    }

    pub fn check_limits(&mut self, stats: Statistics) -> Result<Option<TerminationReason>> {
        self.0.check_limits(stats)
    }

    pub fn current_stats(&self) -> Result<Statistics> {
        self.0.current_stats()
    }
}

impl RunnerController {
    fn send(&self, m: Message) {
        let _ = self.0.send(m);
    }

    pub fn terminate(&self) {
        self.send(Message::Terminate)
    }

    pub fn suspend(&self) {
        self.send(Message::Suspend)
    }

    pub fn resume(&self) {
        self.send(Message::Resume)
    }

    pub fn reset_timers(&self) {
        self.send(Message::ResetTimers)
    }
}

impl RunnerThread {
    pub fn spawn(
        task: Task,
        ps: Process,
        mut on_terminate: Option<Box<OnTerminate>>,
    ) -> Result<Self> {
        let (sender, receiver) = channel();
        thread::Builder::new()
            .spawn(move || {
                let result = RunnerThread::entry(task, ps, receiver);
                if let Some(handler) = on_terminate.as_mut() {
                    handler.on_terminate();
                }
                result
            })
            .map_err(|e| Error::from(e))
            .map(|handle| RunnerThread {
                handle: handle,
                ctl: RunnerController(sender),
            })
    }

    pub fn controller(&self) -> RunnerController {
        self.ctl.clone()
    }

    pub fn join(self) -> Result<RunnerReport> {
        self.handle
            .join()
            .unwrap_or(Err(Error::from("Runner thread panicked")))
    }

    fn entry(task: Task, mut ps: Process, receiver: Receiver<Message>) -> Result<RunnerReport> {
        let mut runner = Runner::new(&mut ps, task.limits);
        let exit_status = RunnerThread::monitor_process(&mut runner, &task, receiver)?;
        let stats = runner.current_stats()?;
        Ok(RunnerReport {
            statistics: stats,
            exit_status: exit_status,
        })
    }

    fn monitor_process(
        runner: &mut Runner,
        task: &Task,
        receiver: Receiver<Message>,
    ) -> Result<ExitStatus> {
        if !task.create_suspended {
            runner.resume_process()?;
        }

        loop {
            if let Some(exit_status) = runner.exit_status()? {
                return Ok(exit_status);
            }

            let stats = runner.current_stats()?;
            if let Some(tr) = runner.check_limits(stats)? {
                return Ok(ExitStatus::Terminated(tr));
            }

            if let Ok(msg) = receiver.try_recv() {
                match msg {
                    Message::Terminate => {
                        return Ok(ExitStatus::Terminated(
                            TerminationReason::ManuallyTerminated,
                        ));
                    }
                    Message::Suspend => runner.suspend_process()?,
                    Message::Resume => runner.resume_process()?,
                    Message::ResetTimers => runner.reset_timers()?,
                }
            }
        }
    }
}
