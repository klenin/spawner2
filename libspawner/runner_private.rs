use command::Command;
use process::{Process, Statistics, Status, Stdio};
use runner::{ExitStatus, Report, Runner, TerminationReason};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub struct WaitHandle {
    monitoring_thread: JoinHandle<io::Result<Report>>,
    runner: Runner,
}

struct MonitoringLoop {
    cmd: Command,
    stats: Statistics,
    exit_status: Option<ExitStatus>,
    is_killed: Arc<AtomicBool>,
}

pub fn run(cmd: Command, stdio: Stdio) -> io::Result<WaitHandle> {
    let monitoring_loop = MonitoringLoop::new(cmd);
    let is_killed = Arc::downgrade(&monitoring_loop.is_killed);

    thread::Builder::new()
        .spawn(move || MonitoringLoop::start(monitoring_loop, stdio))
        .map(|handle| WaitHandle {
            monitoring_thread: handle,
            runner: Runner {
                is_killed: is_killed,
            },
        })
}

impl WaitHandle {
    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn wait(self) -> io::Result<Report> {
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
            stats: Statistics::zeroed(),
            exit_status: None,
            is_killed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn check_limits(&mut self) -> bool {
        let limits = &self.cmd.limits;
        let stats = &self.stats;
        let term_reason = if stats.total_processes > limits.max_processes {
            TerminationReason::Other
        } else if stats.total_user_time > limits.max_user_time {
            TerminationReason::UserTimeLimitExceeded
        } else if stats.total_bytes_written > limits.max_output_size {
            TerminationReason::WriteLimitExceeded
        } else if stats.peak_memory_used > limits.max_memory_usage {
            TerminationReason::MemoryLimitExceeded
        } else {
            return false;
        };

        self.exit_status = Some(ExitStatus::Terminated(term_reason));
        true
    }

    fn should_terminate(&mut self, process: &Process) -> io::Result<bool> {
        match process.status()? {
            Status::Alive(stats) => {
                self.stats = stats;
                Ok(self.check_limits())
            }
            Status::Finished(code) => {
                self.exit_status = Some(ExitStatus::Normal(code));
                Ok(true)
            }
        }
    }

    fn start(mut self, stdio: Stdio) -> io::Result<Report> {
        let process = Process::spawn(&self.cmd, stdio)?;
        while !self.is_killed.load(Ordering::SeqCst) {
            match self.should_terminate(&process) {
                Ok(terminate) => {
                    if terminate {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
            thread::sleep(self.cmd.monitor_interval);
        }

        Ok(Report {
            command: self.cmd,
            statistics: self.stats,
            exit_status: self
                .exit_status
                .unwrap_or(ExitStatus::Terminated(TerminationReason::Other)),
        })
    }
}
