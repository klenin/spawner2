use crate::{Error, Result};
use command::Command;
use process::{Process, Statistics, Status, Stdio};
use runner::{ExitStatus, Report, Runner, TerminationReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct WaitHandle {
    monitoring_thread: JoinHandle<Result<Report>>,
    runner: Runner,
}

struct MonitoringLoop {
    cmd: Command,
    stats: Statistics,
    process_creation_time: Instant,
    last_check_time: Option<Instant>,
    total_idle_time: Duration,
    exit_status: Option<ExitStatus>,
    is_killed: Arc<AtomicBool>,
}

pub fn run(cmd: Command, stdio: Stdio) -> Result<WaitHandle> {
    let monitoring_loop = MonitoringLoop::new(cmd);
    let is_killed = Arc::downgrade(&monitoring_loop.is_killed);

    thread::Builder::new()
        .spawn(move || MonitoringLoop::start(monitoring_loop, stdio))
        .map_err(|e| Error::from(e))
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

    pub fn wait(self) -> Result<Report> {
        match self.monitoring_thread.join() {
            Ok(result) => result,
            Err(_) => Err(Error::from("monitoring thread panicked")),
        }
    }
}

impl MonitoringLoop {
    fn new(cmd: Command) -> Self {
        Self {
            cmd: cmd,
            stats: Statistics::zeroed(),
            process_creation_time: Instant::now(),
            last_check_time: None,
            total_idle_time: Duration::from_millis(0),
            exit_status: None,
            is_killed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn check_limits(&mut self, new_stats: Statistics) -> bool {
        if let Some(last_check_time) = self.last_check_time {
            let dt = last_check_time.elapsed();
            let mut d_user = new_stats.total_user_time - self.stats.total_user_time;
            if d_user > dt {
                d_user = dt;
            }
            self.total_idle_time += dt - d_user;
        }
        self.last_check_time = Some(Instant::now());
        self.stats = new_stats;

        let limits = &self.cmd.limits;
        let term_reason = if self.process_creation_time.elapsed() > limits.max_wall_clock_time {
            TerminationReason::WallClockTimeLimitExceeded
        } else if self.total_idle_time > limits.max_idle_time {
            TerminationReason::IdleTimeLimitExceeded
        } else if self.stats.total_user_time > limits.max_user_time {
            TerminationReason::UserTimeLimitExceeded
        } else if self.stats.total_bytes_written > limits.max_output_size {
            TerminationReason::WriteLimitExceeded
        } else if self.stats.peak_memory_used > limits.max_memory_usage {
            TerminationReason::MemoryLimitExceeded
        } else if self.stats.total_processes > limits.max_processes {
            TerminationReason::ProcessLimitExceeded
        } else {
            return false;
        };

        self.exit_status = Some(ExitStatus::Terminated(term_reason));
        true
    }

    fn should_terminate(&mut self, process: &Process) -> Result<bool> {
        match process.status()? {
            Status::Alive(stats) => Ok(self.check_limits(stats)),
            Status::Finished(code) => {
                self.exit_status = Some(ExitStatus::Normal(code));
                Ok(true)
            }
        }
    }

    fn start(mut self, stdio: Stdio) -> Result<Report> {
        let process = Process::spawn(&self.cmd, stdio)?;
        self.process_creation_time = Instant::now();

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
