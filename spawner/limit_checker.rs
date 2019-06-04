use crate::process::{
    Group, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers, OsLimit,
};
use crate::spawner::{ResourceLimits, TerminationReason};
use crate::Result;

use std::time::{Duration, Instant};

pub struct EnabledOsLimits {
    pub memory: bool,
    pub active_process: bool,
}

pub struct LimitChecker {
    limits: ResourceLimits,
    enabled_os_limits: EnabledOsLimits,
    prev_check: Option<PrevCheck>,
    wall_clock_time: Duration,
    total_user_time: Duration,
    total_idle_time: Duration,
    average_cpu_load: f64,
    average_cpu_load_points: usize,
    time_accounting_stopped: bool,
}

struct PrevCheck {
    time: Instant,
    total_user_time: Duration,
}

struct ResourceUsage {
    memory: GroupMemory,
    timers: GroupTimers,
    io: GroupIo,
    network: GroupNetwork,
    pid_counters: GroupPidCounters,
}

const CPU_LOAD_WINDOW_LENGTH: usize = 20;
const CPU_LOAD_SMOOTHING_FACTOR: f64 = 1.0 - 1.0 / CPU_LOAD_WINDOW_LENGTH as f64;

impl LimitChecker {
    pub fn new(limits: ResourceLimits, enabled_os_limits: EnabledOsLimits) -> Self {
        Self {
            limits: limits,
            enabled_os_limits: enabled_os_limits,
            prev_check: None,
            wall_clock_time: Duration::from_millis(0),
            total_user_time: Duration::from_millis(0),
            total_idle_time: Duration::from_millis(0),
            average_cpu_load: 0.0,
            average_cpu_load_points: 0,
            time_accounting_stopped: false,
        }
    }

    pub fn stop_time_accounting(&mut self) {
        self.time_accounting_stopped = true;
    }

    pub fn resume_time_accounting(&mut self) {
        self.time_accounting_stopped = false;
    }

    pub fn reset_wallclock_and_user_time(&mut self) {
        self.wall_clock_time = Duration::from_millis(0);
        self.total_user_time = Duration::from_millis(0);
    }

    pub fn check(&mut self, group: &mut Group) -> Result<Option<TerminationReason>> {
        if let Some(tr) = self.check_os_limits(group)? {
            return Ok(Some(tr));
        }

        let usage = ResourceUsage::new(group, &self.limits, &self.enabled_os_limits)?;
        self.update_timers(&usage);
        self.prev_check = Some(PrevCheck {
            time: Instant::now(),
            total_user_time: usage.timers.total_user_time,
        });

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        let limits = &self.limits;
        Ok(Some(if gr(self.wall_clock_time, limits.wall_clock_time) {
            TerminationReason::WallClockTimeLimitExceeded
        } else if gr(
            self.total_idle_time,
            limits.idle_time.map(|i| i.total_idle_time),
        ) {
            TerminationReason::IdleTimeLimitExceeded
        } else if gr(self.total_user_time, limits.total_user_time) {
            TerminationReason::UserTimeLimitExceeded
        } else if gr(usage.io.total_bytes_written, limits.total_bytes_written) {
            TerminationReason::WriteLimitExceeded
        } else if gr(usage.memory.max_usage, limits.max_memory_usage) {
            TerminationReason::MemoryLimitExceeded
        } else if gr(
            usage.pid_counters.total_processes,
            limits.total_processes_created,
        ) {
            TerminationReason::ProcessLimitExceeded
        } else if gr(usage.pid_counters.active_processes, limits.active_processes) {
            TerminationReason::ActiveProcessLimitExceeded
        } else if gr(
            usage.network.active_connections,
            limits.active_network_connections,
        ) {
            TerminationReason::ActiveNetworkConnectionLimitExceeded
        } else {
            return Ok(None);
        }))
    }

    fn update_timers(&mut self, usage: &ResourceUsage) {
        if self.time_accounting_stopped {
            return;
        }

        let prev_check = match self.prev_check {
            Some(ref prev_check) => prev_check,
            None => return,
        };
        let dt = prev_check.time.elapsed();
        let d_user = usage.timers.total_user_time - prev_check.total_user_time;
        let new_cpu_load = d_user.as_micros() as f64 / dt.as_micros() as f64;

        self.wall_clock_time += dt;
        self.total_user_time += d_user;
        self.average_cpu_load = self.average_cpu_load * CPU_LOAD_SMOOTHING_FACTOR
            + new_cpu_load * (1.0 - CPU_LOAD_SMOOTHING_FACTOR);
        self.average_cpu_load_points += 1;

        let idle_time_limit = match self.limits.idle_time {
            Some(il) => il,
            None => return,
        };
        if self.average_cpu_load_points < CPU_LOAD_WINDOW_LENGTH {
            return;
        }
        if self.average_cpu_load < idle_time_limit.cpu_load_threshold {
            self.total_idle_time += dt;
        } else {
            self.total_idle_time = Duration::from_millis(0);
        }
    }

    fn check_os_limits(&mut self, group: &mut Group) -> Result<Option<TerminationReason>> {
        if self.enabled_os_limits.memory && group.is_os_limit_hit(OsLimit::Memory)? {
            return Ok(Some(TerminationReason::MemoryLimitExceeded));
        }
        if self.enabled_os_limits.active_process && group.is_os_limit_hit(OsLimit::ActiveProcess)? {
            return Ok(Some(TerminationReason::ActiveProcessLimitExceeded));
        }
        Ok(None)
    }
}

impl ResourceUsage {
    fn new(
        group: &mut Group,
        limits: &ResourceLimits,
        enabled_os_limits: &EnabledOsLimits,
    ) -> Result<ResourceUsage> {
        let query_timers = limits.idle_time.is_some() || limits.total_user_time.is_some();
        let query_memory = !enabled_os_limits.memory && limits.max_memory_usage.is_some();
        let query_io = limits.total_bytes_written.is_some();
        let query_network = limits.active_network_connections.is_some();
        let query_pid_counters = (!enabled_os_limits.active_process
            && limits.active_processes.is_some())
            || limits.total_processes_created.is_some();

        Ok(ResourceUsage {
            timers: if query_timers { group.timers()? } else { None }.unwrap_or_default(),
            memory: if query_memory { group.memory()? } else { None }.unwrap_or_default(),
            io: if query_io { group.io()? } else { None }.unwrap_or_default(),
            network: if query_network {
                group.network()?
            } else {
                None
            }
            .unwrap_or_default(),
            pid_counters: if query_pid_counters {
                group.pid_counters()?
            } else {
                None
            }
            .unwrap_or_default(),
        })
    }
}
