use crate::command::Command;
use crate::sys::runner::RunnerMessage;

use std::sync::mpsc::Sender;
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
    Crashed(u32, &'static str),
    Finished(u32),
    Terminated(TerminationReason),
}

#[derive(Clone, Debug)]
pub struct RunnerReport {
    pub command: Command,
    pub statistics: Statistics,
    pub exit_status: ExitStatus,
}

#[derive(Clone)]
pub struct Runner(Sender<RunnerMessage>);

impl Runner {
    fn send(&self, msg: RunnerMessage) {
        let _ = self.0.send(msg);
    }

    pub fn terminate(&self) {
        self.send(RunnerMessage::Terminate);
    }

    pub fn suspend(&self) {
        self.send(RunnerMessage::Suspend);
    }

    pub fn resume(&self) {
        self.send(RunnerMessage::Resume);
    }

    pub fn reset_timers(&self) {
        self.send(RunnerMessage::ResetTimers);
    }
}

impl From<Sender<RunnerMessage>> for Runner {
    fn from(s: Sender<RunnerMessage>) -> Self {
        Self(s)
    }
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
