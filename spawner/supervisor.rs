use crate::limit_checker::LimitChecker;
use crate::process::{Group, OsLimit, Process, ProcessInfo, ResourceUsage, Stdio};
use crate::{ProgramMessage, Report, ResourceLimits, Result, TerminationReason};

use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

pub struct Supervisor {
    limit_checker: LimitChecker,
    process: Process,
    creation_time: Instant,
    term_reason: Option<TerminationReason>,
    msg_receiver: Option<Receiver<ProgramMessage>>,
    monitor_interval: Duration,
    wait_for_children: bool,
}

impl Supervisor {
    pub fn start_monitoring(
        info: ProcessInfo,
        stdio: Stdio,
        mut group: Group,
        limits: ResourceLimits,
        monitor_interval: Duration,
        receiver: Option<Receiver<ProgramMessage>>,
        wait_for_children: bool,
    ) -> Result<Report> {
        if let Some(mem_limit) = limits.max_memory_usage {
            group.set_os_limit(OsLimit::Memory, mem_limit)?;
        }
        if let Some(num) = limits.active_processes {
            group.set_os_limit(OsLimit::ActiveProcess, num as u64)?;
        }

        Process::spawn_in_group(info, stdio, &mut group)
            .map(|ps| Self {
                limit_checker: LimitChecker::new(limits),
                process: ps,
                creation_time: Instant::now(),
                term_reason: None,
                msg_receiver: receiver,
                monitor_interval,
                wait_for_children,
            })
            .and_then(|pm| pm.monitoring_loop(group))
    }

    fn monitoring_loop(mut self, group: Group) -> Result<Report> {
        let mut usage = ResourceUsage::new(&group);
        let mut last_check_time = Instant::now();
        loop {
            usage.update()?;
            if let Some(report) = self.get_report(&group, &usage)? {
                return Ok(report);
            }

            if last_check_time.elapsed() > self.monitor_interval {
                last_check_time = Instant::now();
                if let Some(tr) = self.check_limits(&group, &usage)? {
                    group.terminate()?;
                    self.term_reason = Some(tr);
                }
            }

            self.handle_messages(&group)?;
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn check_limits(
        &mut self,
        group: &Group,
        usage: &ResourceUsage,
    ) -> Result<Option<TerminationReason>> {
        if group.is_os_limit_hit(OsLimit::Memory)? {
            return Ok(Some(TerminationReason::MemoryLimitExceeded));
        }
        if group.is_os_limit_hit(OsLimit::ActiveProcess)? {
            return Ok(Some(TerminationReason::ActiveProcessLimitExceeded));
        }
        self.limit_checker.check(usage)
    }

    fn get_report(&mut self, group: &Group, usage: &ResourceUsage) -> Result<Option<Report>> {
        let exit_status = match self.process.exit_status()? {
            Some(status) => status,
            None => return Ok(None),
        };

        let pid_counters = usage.pid_counters()?;
        if self.wait_for_children
            && pid_counters.is_some()
            && pid_counters.unwrap().active_processes != 0
        {
            return Ok(None);
        }

        if self.term_reason.is_none() {
            self.term_reason = self.check_limits(group, usage)?;
        }

        Ok(Some(Report {
            wall_clock_time: self.creation_time.elapsed(),
            memory: usage.memory()?,
            io: usage.io()?,
            timers: usage.timers()?,
            pid_counters,
            network: usage.network()?,
            exit_status,
            termination_reason: self.term_reason,
        }))
    }

    fn handle_messages(&mut self, group: &Group) -> Result<()> {
        let receiver = match &mut self.msg_receiver {
            Some(r) => r,
            None => return Ok(()),
        };
        for msg in receiver.try_iter().take(10) {
            match msg {
                ProgramMessage::Terminate => {
                    group.terminate()?;
                    self.term_reason = Some(TerminationReason::TerminatedByRunner);
                }
                ProgramMessage::Suspend => {
                    if self.process.exit_status()?.is_none() {
                        self.process.suspend()?;
                    }
                }
                ProgramMessage::Resume => {
                    if self.process.exit_status()?.is_none() {
                        self.process.resume()?;
                    }
                }
                ProgramMessage::ResetTime => self.limit_checker.reset_time(),
                ProgramMessage::StopTimeAccounting => self.limit_checker.stop_time_accounting(),
                ProgramMessage::ResumeTimeAccounting => self.limit_checker.resume_time_accounting(),
            }
        }
        Ok(())
    }
}
