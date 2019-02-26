use crate::{Error, Result};
use command::{Command, OnTerminate};
use process::{Process, ProcessInfo, ProcessStatus, ProcessStdio};
use runner::{ExitStatus, Runner, RunnerReport, TerminationReason};
use std::sync::mpsc::{channel, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct RunnerThread {
    handle: JoinHandle<Result<RunnerReport>>,
    runner: Runner,
}

pub enum Message {
    Terminate,
    Suspend,
    Resume,
    ResetTimers,
}

struct RunnerImpl {
    cmd: Command,
    info: ProcessInfo,
    last_check_time: Option<Instant>,
    total_idle_time: Duration,
    exit_status: Option<ExitStatus>,
    receiver: Receiver<Message>,
}

struct OnTerminateGuard {
    handler: Option<Box<OnTerminate>>,
}

pub fn spawn(
    cmd: Command,
    on_terminate: Option<Box<OnTerminate>>,
    stdio: ProcessStdio,
) -> Result<RunnerThread> {
    let (sender, receiver) = channel::<Message>();
    let monitoring_loop = RunnerImpl::new(cmd, receiver);

    thread::Builder::new()
        .spawn(move || RunnerImpl::main_loop(monitoring_loop, on_terminate, stdio))
        .map_err(|e| Error::from(e))
        .map(|handle| RunnerThread {
            handle: handle,
            runner: Runner { sender: sender },
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
    fn new(cmd: Command, receiver: Receiver<Message>) -> Self {
        Self {
            cmd: cmd,
            info: ProcessInfo::zeroed(),
            last_check_time: None,
            total_idle_time: Duration::from_millis(0),
            exit_status: None,
            receiver: receiver,
        }
    }

    fn check_limits(&mut self, new_info: ProcessInfo) -> bool {
        if let Some(last_check_time) = self.last_check_time {
            let dt = last_check_time.elapsed();
            let mut d_user = new_info.total_user_time - self.info.total_user_time;
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
        self.info = new_info;

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        let limits = &self.cmd.limits;
        let term_reason = if gr(self.info.wall_clock_time, limits.max_wall_clock_time) {
            TerminationReason::WallClockTimeLimitExceeded
        } else if gr(self.total_idle_time, limits.max_idle_time) {
            TerminationReason::IdleTimeLimitExceeded
        } else if gr(self.info.total_user_time, limits.max_user_time) {
            TerminationReason::UserTimeLimitExceeded
        } else if gr(self.info.total_bytes_written, limits.max_output_size) {
            TerminationReason::WriteLimitExceeded
        } else if gr(self.info.peak_memory_used, limits.max_memory_usage) {
            TerminationReason::MemoryLimitExceeded
        } else if gr(self.info.total_processes, limits.max_processes) {
            TerminationReason::ProcessLimitExceeded
        } else {
            return false;
        };

        self.exit_status = Some(ExitStatus::Terminated(term_reason));
        true
    }

    fn main_loop(
        mut self,
        on_terminate: Option<Box<OnTerminate>>,
        stdio: ProcessStdio,
    ) -> Result<RunnerReport> {
        let _on_terminate_guard = OnTerminateGuard::new(on_terminate);
        let process = Process::spawn(&self.cmd, stdio)?;

        loop {
            match process.status()? {
                ProcessStatus::Running => {
                    if self.check_limits(process.info()?) {
                        break;
                    }
                }
                ProcessStatus::Finished(code) => {
                    self.exit_status = Some(ExitStatus::Finished(code));
                    break;
                }
                ProcessStatus::Crashed(status_crashed) => {
                    self.exit_status = Some(ExitStatus::Crashed(status_crashed));
                    break;
                }
            }

            if let Ok(msg) = self.receiver.try_recv() {
                match msg {
                    Message::Terminate => {
                        self.exit_status = Some(ExitStatus::Terminated(TerminationReason::Other));
                        break;
                    }
                    Message::Suspend => process.suspend()?,
                    Message::Resume => process.resume()?,
                    Message::ResetTimers => {
                        self.info.wall_clock_time = Duration::from_millis(0);
                        self.info.total_user_time = Duration::from_millis(0);
                    }
                }
            }

            thread::sleep(self.cmd.monitor_interval);
        }

        Ok(RunnerReport {
            command: self.cmd,
            process_info: self.info,
            exit_status: self.exit_status.unwrap(),
        })
    }
}

impl OnTerminateGuard {
    fn new(handler: Option<Box<OnTerminate>>) -> Self {
        Self { handler: handler }
    }
}

impl Drop for OnTerminateGuard {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.as_mut() {
            handler.on_terminate();
        }
    }
}
