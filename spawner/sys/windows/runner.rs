use crate::{Error, Result};
use command::{Command, OnTerminate};
use runner::{ExitStatus, ProcessInfo, Runner, RunnerReport, TerminationReason};
use std::sync::mpsc::{channel, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sys::windows::pipe::{ReadPipe, WritePipe};
use sys::windows::process::{Process, RawStdio, Status};
use sys::IntoInner;

pub struct RunnerThread {
    handle: JoinHandle<Result<RunnerReport>>,
    runner: Runner,
}

pub enum RunnerMessage {
    Terminate,
    Suspend,
    Resume,
    ResetTimers,
}

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

struct MonitoringLoop {
    cmd: Command,
    process: Process,
    ps_info: ProcessInfo,
    last_check_time: Option<Instant>,
    total_idle_time: Duration,
    exit_status: Option<ExitStatus>,
    receiver: Receiver<RunnerMessage>,
}

pub fn spawn(
    cmd: Command,
    stdio: ProcessStdio,
    mut on_terminate: Option<Box<OnTerminate>>,
) -> Result<RunnerThread> {
    let (sender, receiver) = channel::<RunnerMessage>();
    thread::Builder::new()
        .spawn(move || {
            let process = Process::spawn(
                &cmd,
                RawStdio {
                    stdin: stdio.stdin.into_inner(),
                    stdout: stdio.stdout.into_inner(),
                    stderr: stdio.stderr.into_inner(),
                },
            )?;

            let monitoring_loop = MonitoringLoop::new(cmd, process, receiver);
            let report = monitoring_loop.run();
            if let Some(handler) = on_terminate.as_mut() {
                handler.on_terminate();
            }
            report
        })
        .map_err(|e| Error::from(e))
        .map(|handle| RunnerThread {
            handle: handle,
            runner: Runner::from(sender),
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

impl MonitoringLoop {
    fn new(cmd: Command, process: Process, receiver: Receiver<RunnerMessage>) -> Self {
        Self {
            cmd: cmd,
            process: process,
            ps_info: ProcessInfo::zeroed(),
            last_check_time: None,
            total_idle_time: Duration::from_millis(0),
            exit_status: None,
            receiver: receiver,
        }
    }

    fn check_limits(&mut self, new_info: ProcessInfo) -> bool {
        if let Some(last_check_time) = self.last_check_time {
            let dt = last_check_time.elapsed();
            let mut d_user = new_info.total_user_time - self.ps_info.total_user_time;
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
        self.ps_info = new_info;

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        let limits = &self.cmd.limits;
        let term_reason = if gr(self.ps_info.wall_clock_time, limits.max_wall_clock_time) {
            TerminationReason::WallClockTimeLimitExceeded
        } else if gr(self.total_idle_time, limits.max_idle_time) {
            TerminationReason::IdleTimeLimitExceeded
        } else if gr(self.ps_info.total_user_time, limits.max_user_time) {
            TerminationReason::UserTimeLimitExceeded
        } else if gr(self.ps_info.total_bytes_written, limits.max_output_size) {
            TerminationReason::WriteLimitExceeded
        } else if gr(self.ps_info.peak_memory_used, limits.max_memory_usage) {
            TerminationReason::MemoryLimitExceeded
        } else if gr(self.ps_info.total_processes_created, limits.max_processes) {
            TerminationReason::ProcessLimitExceeded
        } else {
            return false;
        };

        self.exit_status = Some(ExitStatus::Terminated(term_reason));
        true
    }

    fn process_info(&self) -> Result<ProcessInfo> {
        let basic_and_io_info = self.process.basic_and_io_info()?;
        let ext_limit_info = self.process.ext_limit_info()?;

        // Total user time in 100-nanosecond ticks.
        let total_user_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalUserTime.QuadPart() } as u64;
        // Total kernel time in 100-nanosecond ticks.
        let total_kernel_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalKernelTime.QuadPart() } as u64;

        Ok(ProcessInfo {
            wall_clock_time: self.process.creation_time().elapsed(),
            total_user_time: Duration::from_nanos(total_user_time * 100),
            total_kernel_time: Duration::from_nanos(total_kernel_time * 100),
            peak_memory_used: ext_limit_info.PeakJobMemoryUsed as u64,
            total_processes_created: basic_and_io_info.BasicInfo.TotalProcesses as usize,
            total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
        })
    }

    fn run(mut self) -> Result<RunnerReport> {
        loop {
            match self.process.status()? {
                Status::Running => {
                    let new_info = self.process_info()?;
                    if self.check_limits(new_info) {
                        break;
                    }
                }
                Status::Finished(code) => {
                    self.exit_status = Some(ExitStatus::Finished(code));
                    break;
                }
                Status::Crashed(code, cause) => {
                    self.exit_status = Some(ExitStatus::Crashed(code, cause));
                    break;
                }
            }

            if let Ok(msg) = self.receiver.try_recv() {
                match msg {
                    RunnerMessage::Terminate => {
                        self.exit_status = Some(ExitStatus::Terminated(
                            TerminationReason::ManuallyTerminated,
                        ));
                        break;
                    }
                    RunnerMessage::Suspend => self.process.suspend()?,
                    RunnerMessage::Resume => self.process.resume()?,
                    RunnerMessage::ResetTimers => {
                        self.ps_info.wall_clock_time = Duration::from_millis(0);
                        self.ps_info.total_user_time = Duration::from_millis(0);
                        self.total_idle_time = Duration::from_millis(0);
                    }
                }
            }

            thread::sleep(self.cmd.monitor_interval);
        }

        Ok(RunnerReport {
            command: self.cmd,
            process_info: self.ps_info,
            exit_status: self.exit_status.unwrap(),
        })
    }
}
