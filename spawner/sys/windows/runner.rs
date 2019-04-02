use crate::command::Command;
use crate::runner::{ExitStatus, Runner, RunnerReport, Statistics, TerminationReason};
use crate::sys::runner_common::LimitChecker;
use crate::sys::windows::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::process::{Process, Status};
use crate::sys::windows::utils::Stdio;
use crate::sys::IntoInner;
use crate::Result;

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

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

pub struct MonitoringLoop {
    cmd: Command,
    process: Process,
    limit_checker: LimitChecker,
    sender: Sender<RunnerMessage>,
    receiver: Receiver<RunnerMessage>,
}

unsafe impl Send for MonitoringLoop {}

impl MonitoringLoop {
    pub fn create(cmd: Command, stdio: ProcessStdio) -> Result<Self> {
        let (sender, receiver) = channel();
        let process = Process::spawn_suspended(
            &cmd,
            Stdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )?;
        let checker = LimitChecker::new(cmd.limits);

        Ok(Self {
            cmd: cmd,
            process: process,
            limit_checker: checker,
            sender: sender,
            receiver: receiver,
        })
    }

    pub fn runner(&self) -> Runner {
        Runner::from(self.sender.clone())
    }

    pub fn entry(mut self) -> Result<RunnerReport> {
        let exit_status = self._loop()?;
        Ok(RunnerReport {
            command: self.cmd,
            statistics: self.limit_checker.stats(),
            exit_status: exit_status,
        })
    }

    fn _loop(&mut self) -> Result<ExitStatus> {
        let creation_time = Instant::now();
        if !self.cmd.create_suspended {
            self.process.resume()?;
        }

        loop {
            match self.process.status()? {
                Status::Running => {
                    let current_stats = self.current_stats(&creation_time)?;
                    if let Some(tr) = self.limit_checker.check(current_stats) {
                        return Ok(ExitStatus::Terminated(tr));
                    }
                }
                Status::Finished(code) => {
                    return Ok(ExitStatus::Finished(code));
                }
                Status::Crashed(code, cause) => {
                    return Ok(ExitStatus::Crashed(code, cause));
                }
            }

            if let Ok(msg) = self.receiver.try_recv() {
                match msg {
                    RunnerMessage::Terminate => {
                        return Ok(ExitStatus::Terminated(
                            TerminationReason::ManuallyTerminated,
                        ));
                    }
                    RunnerMessage::Suspend => self.process.suspend()?,
                    RunnerMessage::Resume => self.process.resume()?,
                    RunnerMessage::ResetTimers => self.limit_checker.reset_timers(),
                }
            }

            thread::sleep(self.cmd.monitor_interval);
        }
    }

    fn current_stats(&self, creation_time: &Instant) -> Result<Statistics> {
        let basic_and_io_info = self.process.basic_and_io_info()?;
        let ext_limit_info = self.process.ext_limit_info()?;

        // Total user time in 100-nanosecond ticks.
        let total_user_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalUserTime.QuadPart() } as u64;

        // Total kernel time in 100-nanosecond ticks.
        let total_kernel_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalKernelTime.QuadPart() } as u64;

        Ok(Statistics {
            wall_clock_time: creation_time.elapsed(),
            total_user_time: Duration::from_nanos(total_user_time * 100),
            total_kernel_time: Duration::from_nanos(total_kernel_time * 100),
            peak_memory_used: ext_limit_info.PeakJobMemoryUsed as u64,
            total_processes_created: basic_and_io_info.BasicInfo.TotalProcesses as usize,
            total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
        })
    }
}
