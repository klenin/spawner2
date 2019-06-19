use crate::limit_checker::{EnabledOsLimits, LimitChecker};
use crate::process::{self, Group, OsLimit, Process, ProcessInfo};
use crate::spawner::{OnTerminate, Report, ResourceLimits, TerminationReason};
use crate::{Error, Result};

use std::sync::mpsc::Receiver;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum RunnerMessage {
    Terminate,
    Suspend,
    Resume,
    StopTimeAccounting,
    ResumeTimeAccounting,
    ResetWallclockAndUserTime,
}

pub struct RunnerData {
    pub info: ProcessInfo,
    pub stdio: process::Stdio,
    pub group: Group,
    pub limits: ResourceLimits,
    pub monitor_interval: Duration,
    pub on_terminate: Option<Box<OnTerminate>>,
    pub receiver: Receiver<RunnerMessage>,
    pub wait_for_children: bool,
}

pub struct RunnerThread(JoinHandle<Result<Report>>);

struct ProcessMonitor {
    limit_checker: LimitChecker,
    process: Process,
    creation_time: Instant,
    term_reason: Option<TerminationReason>,
    group: Group,
    receiver: Receiver<RunnerMessage>,
    monitor_interval: Duration,
    wait_for_children: bool,
    on_terminate: Option<Box<OnTerminate>>,
}

impl RunnerThread {
    pub fn spawn(data: RunnerData) -> Self {
        Self(thread::spawn(move || {
            ProcessMonitor::new(data).and_then(|mut pm| pm.start_monitoring())
        }))
    }

    pub fn join(self) -> Result<Report> {
        self.0
            .join()
            .unwrap_or(Err(Error::from("RunnerThread panicked")))
    }
}

impl ProcessMonitor {
    fn new(mut data: RunnerData) -> Result<Self> {
        let limit_checker = LimitChecker::new(
            data.limits,
            EnabledOsLimits {
                memory: data
                    .limits
                    .max_memory_usage
                    .map(|limit| data.group.set_os_limit(OsLimit::Memory, limit))
                    .transpose()?
                    .unwrap_or(false),
                active_process: data
                    .limits
                    .active_processes
                    .map(|limit| {
                        data.group
                            .set_os_limit(OsLimit::ActiveProcess, limit as u64)
                    })
                    .transpose()?
                    .unwrap_or(false),
            },
        );

        let ps = Process::spawn_in_group(data.info, data.stdio, &mut data.group)?;
        Ok(Self {
            limit_checker: limit_checker,
            process: ps,
            creation_time: Instant::now(),
            term_reason: None,
            group: data.group,
            receiver: data.receiver,
            monitor_interval: data.monitor_interval,
            wait_for_children: data.wait_for_children,
            on_terminate: data.on_terminate,
        })
    }

    fn start_monitoring(&mut self) -> Result<Report> {
        loop {
            if let Some(report) = self.get_report()? {
                return Ok(report);
            }
            if let Some(tr) = self.limit_checker.check(&mut self.group)? {
                self.group.terminate()?;
                self.term_reason = Some(tr);
            }
            self.handle_messages()?;
            thread::sleep(self.monitor_interval);
        }
    }

    fn get_report(&mut self) -> Result<Option<Report>> {
        let exit_status = match self.process.exit_status()? {
            Some(status) => status,
            None => return Ok(None),
        };

        let pid_counters = self.group.pid_counters()?;

        if self.wait_for_children
            && pid_counters.is_some()
            && pid_counters.unwrap().active_processes != 0
        {
            return Ok(None);
        }

        if self.term_reason.is_none() {
            self.term_reason = self.limit_checker.check(&mut self.group)?;
        }

        return Ok(Some(Report {
            wall_clock_time: self.creation_time.elapsed(),
            memory: self.group.memory()?,
            io: self.group.io()?,
            timers: self.group.timers()?,
            pid_counters: pid_counters,
            network: self.group.network()?,
            exit_status: exit_status,
            termination_reason: self.term_reason,
        }));
    }

    fn handle_messages(&mut self) -> Result<()> {
        for msg in self.receiver.try_iter().take(10) {
            match msg {
                RunnerMessage::Terminate => {
                    self.group.terminate()?;
                    self.term_reason = Some(TerminationReason::TerminatedByRunner);
                }
                RunnerMessage::Suspend => {
                    if self.process.exit_status()?.is_none() {
                        self.process.suspend()?;
                    }
                }
                RunnerMessage::Resume => {
                    if self.process.exit_status()?.is_none() {
                        self.process.resume()?;
                    }
                }
                RunnerMessage::ResetWallclockAndUserTime => {
                    self.limit_checker.reset_wallclock_and_user_time()
                }
                RunnerMessage::StopTimeAccounting => self.limit_checker.stop_time_accounting(),
                RunnerMessage::ResumeTimeAccounting => self.limit_checker.resume_time_accounting(),
            }
        }

        Ok(())
    }
}

impl Drop for ProcessMonitor {
    fn drop(&mut self) {
        let _ = self.group.terminate();
        if let Some(mut handler) = self.on_terminate.take() {
            handler.on_terminate();
        }
    }
}
