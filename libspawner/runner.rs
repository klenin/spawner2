use command::Command;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::u64;
use sys::process::{ProcessTree, ProcessTreeStatus, SummaryInfo};

#[derive(Copy, Clone)]
pub enum TerminationReason {
    None,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    Other,
}

#[derive(Clone)]
pub struct Report {
    pub cmd: Command,
    pub termination_reason: TerminationReason,
    pub user_time: Duration,
    pub kernel_time: Duration,
    pub peak_memory_used: u64,
    pub processes_created: u64,
    pub exit_code: i32,
}

#[derive(Clone)]
pub struct Runner {
    is_killed: Weak<AtomicBool>,
}

pub(crate) struct WaitHandle {
    monitoring_thread: JoinHandle<io::Result<Report>>,
    runner: Runner,
}

struct MonitoringLoop {
    cmd: Command,
    is_killed: Arc<AtomicBool>,
}

pub(crate) fn run(cmd: Command) -> io::Result<WaitHandle> {
    let monitoring_loop = MonitoringLoop::new(cmd);
    let is_killed = Arc::downgrade(&monitoring_loop.is_killed);

    thread::Builder::new()
        .spawn(move || MonitoringLoop::start(monitoring_loop))
        .map(|handle| WaitHandle {
            monitoring_thread: handle,
            runner: Runner {
                is_killed: is_killed,
            },
        })
}

impl Runner {
    pub fn kill(&self) {
        if let Some(flag) = self.is_killed.upgrade() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

impl WaitHandle {
    pub(crate) fn runner(&self) -> &Runner {
        &self.runner
    }

    pub(crate) fn wait(self) -> io::Result<Report> {
        match self.monitoring_thread.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::Other,
                "monitoring thread panicked",
            )),
        }
    }
}

impl MonitoringLoop {
    fn new(cmd: Command) -> Self {
        Self {
            cmd: cmd,
            is_killed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn termination_reason(&self, info: &SummaryInfo) -> TerminationReason {
        let limits = &self.cmd.limits;
        if info.total_processes > limits.max_processes {
            TerminationReason::Other
        } else if info.total_user_time > limits.max_user_time {
            TerminationReason::UserTimeLimitExceeded
        } else if info.total_bytes_written > limits.max_output_size {
            TerminationReason::WriteLimitExceeded
        } else if info.peak_memory_used > limits.max_memory_usage {
            TerminationReason::MemoryLimitExceeded
        } else {
            TerminationReason::None
        }
    }

    fn start(self) -> io::Result<Report> {
        let mut summary_info = SummaryInfo::zeroed();
        let mut termination_reason = TerminationReason::None;
        let mut exit_code = 0;
        let pstree = ProcessTree::spawn(&self.cmd.inner)?;

        while !self.is_killed.load(Ordering::SeqCst) {
            match pstree.status() {
                Ok(status) => match status {
                    ProcessTreeStatus::Alive(info) => {
                        summary_info = info;
                        termination_reason = self.termination_reason(&summary_info);
                        match termination_reason {
                            TerminationReason::None => {}
                            _ => break,
                        }
                    }
                    ProcessTreeStatus::Finished(c) => {
                        exit_code = c;
                        break;
                    }
                },
                Err(e) => {
                    pstree.kill()?;
                    return Err(e);
                }
            }
            thread::sleep(self.cmd.monitor_interval);
        }

        pstree.kill()?;
        Ok(Report {
            cmd: self.cmd,
            termination_reason: termination_reason,
            user_time: summary_info.total_user_time,
            kernel_time: summary_info.total_kernel_time,
            peak_memory_used: summary_info.peak_memory_used,
            processes_created: summary_info.total_processes,
            exit_code: exit_code,
        })
    }
}
