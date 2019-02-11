use crate::{Error, Result};
use command::Command;
use process::{Process, Statistics, Status, Stdio};
use runner::{ExitStatus, Runner, RunnerReport, TerminationReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct RunnerThread {
    handle: JoinHandle<Result<RunnerReport>>,
    runner: Runner,
}

struct RunnerImpl {
    cmd: Command,
    stats: Statistics,
    last_check_time: Option<Instant>,
    total_idle_time: Duration,
    exit_status: Option<ExitStatus>,
    is_killed: Arc<AtomicBool>,
}

pub fn spawn(cmd: Command, stdio: Stdio) -> Result<RunnerThread> {
    let monitoring_loop = RunnerImpl::new(cmd);
    let is_killed = Arc::downgrade(&monitoring_loop.is_killed);

    thread::Builder::new()
        .spawn(move || RunnerImpl::spawn(monitoring_loop, stdio))
        .map_err(|e| Error::from(e))
        .map(|handle| RunnerThread {
            handle: handle,
            runner: Runner {
                is_killed: is_killed,
            },
        })
}

impl RunnerThread {
    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn join(self) -> Result<RunnerReport> {
        match self.handle.join() {
            Ok(result) => result,
            Err(_) => Err(Error::from("monitoring thread panicked")),
        }
    }
}

impl RunnerImpl {
    fn new(cmd: Command) -> Self {
        Self {
            cmd: cmd,
            stats: Statistics::zeroed(),
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
            // FIXME: total_user_time contains user times of all processes created, therefore
            // it can be greater than the wall-clock time. Currently it is possible for 2 processes
            // to avoid idle time limit. Consider:
            // First process:
            //     int main() { while (1) { } }
            // Second process:
            //     int main() { sleep(1000000); }
            //
            // In this case d_user will be equal to dt, therefore 0 idle time will be added.
            // One way to fix this is computing the idle time for each active process e.g:
            // total_idle_time += dt * active_procesess - user_time_of_all_active_processes
            if d_user > dt {
                d_user = dt;
            }
            self.total_idle_time += dt - d_user;
        }
        self.last_check_time = Some(Instant::now());
        self.stats = new_stats;

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        let limits = &self.cmd.limits;
        let term_reason = if gr(self.stats.wall_clock_time, limits.max_wall_clock_time) {
            TerminationReason::WallClockTimeLimitExceeded
        } else if gr(self.total_idle_time, limits.max_idle_time) {
            TerminationReason::IdleTimeLimitExceeded
        } else if gr(self.stats.total_user_time, limits.max_user_time) {
            TerminationReason::UserTimeLimitExceeded
        } else if gr(self.stats.total_bytes_written, limits.max_output_size) {
            TerminationReason::WriteLimitExceeded
        } else if gr(self.stats.peak_memory_used, limits.max_memory_usage) {
            TerminationReason::MemoryLimitExceeded
        } else if gr(self.stats.total_processes, limits.max_processes) {
            TerminationReason::ProcessLimitExceeded
        } else {
            return false;
        };

        self.exit_status = Some(ExitStatus::Terminated(term_reason));
        true
    }

    fn spawn(mut self, stdio: Stdio) -> Result<RunnerReport> {
        let process = Process::spawn(&self.cmd, stdio)?;

        loop {
            if self.is_killed.load(Ordering::SeqCst) {
                self.exit_status = Some(ExitStatus::Terminated(TerminationReason::Other));
                break;
            }

            match process.status()? {
                Status::Alive(stats) => {
                    if self.check_limits(stats) {
                        break;
                    }
                }
                Status::Finished(code) => {
                    self.exit_status = Some(ExitStatus::Finished(code));
                    break;
                }
            }
            thread::sleep(self.cmd.monitor_interval);
        }

        Ok(RunnerReport {
            command: self.cmd,
            statistics: self.stats,
            exit_status: self.exit_status.unwrap(),
        })
    }
}
