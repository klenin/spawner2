use crate::process::{GroupTimers, ResourceUsage};
use crate::spawner::{ResourceLimits, TerminationReason};
use crate::Result;

use std::time::{Duration, Instant};

pub struct LimitChecker {
    limits: ResourceLimits,
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

const CPU_LOAD_WINDOW_LENGTH: usize = 20;
const CPU_LOAD_SMOOTHING_FACTOR: f64 = 1.0 - 1.0 / CPU_LOAD_WINDOW_LENGTH as f64;

impl LimitChecker {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
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

    pub fn reset_time(&mut self) {
        self.wall_clock_time = Duration::from_millis(0);
        self.total_user_time = Duration::from_millis(0);
    }

    pub fn check(&mut self, usage: &ResourceUsage) -> Result<Option<TerminationReason>> {
        let timers = usage.timers()?.unwrap_or_default();
        self.update_timers(timers);
        self.prev_check = Some(PrevCheck {
            time: Instant::now(),
            total_user_time: timers.total_user_time,
        });

        let limits = &self.limits;
        let query_memory = limits.max_memory_usage.is_some();
        let query_io = limits.total_bytes_written.is_some();
        let query_network = limits.active_network_connections.is_some();
        let query_pid_counters =
            limits.active_processes.is_some() || limits.total_processes_created.is_some();

        let memory = if query_memory { usage.memory()? } else { None }.unwrap_or_default();
        let io = if query_io { usage.io()? } else { None }.unwrap_or_default();
        let network = if query_network {
            usage.network()?
        } else {
            None
        }
        .unwrap_or_default();
        let pid_counters = if query_pid_counters {
            usage.pid_counters()?
        } else {
            None
        }
        .unwrap_or_default();

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        Ok(Some(if gr(self.wall_clock_time, limits.wall_clock_time) {
            TerminationReason::WallClockTimeLimitExceeded
        } else if gr(
            self.total_idle_time,
            limits.idle_time.map(|i| i.total_idle_time),
        ) {
            TerminationReason::IdleTimeLimitExceeded
        } else if gr(self.total_user_time, limits.total_user_time) {
            TerminationReason::UserTimeLimitExceeded
        } else if gr(io.total_bytes_written, limits.total_bytes_written) {
            TerminationReason::WriteLimitExceeded
        } else if gr(memory.max_usage, limits.max_memory_usage) {
            TerminationReason::MemoryLimitExceeded
        } else if gr(pid_counters.total_processes, limits.total_processes_created) {
            TerminationReason::ProcessLimitExceeded
        } else if gr(pid_counters.active_processes, limits.active_processes) {
            TerminationReason::ActiveProcessLimitExceeded
        } else if gr(
            network.active_connections,
            limits.active_network_connections,
        ) {
            TerminationReason::ActiveNetworkConnectionLimitExceeded
        } else {
            return Ok(None);
        }))
    }

    fn update_timers(&mut self, timers: GroupTimers) {
        if self.time_accounting_stopped {
            return;
        }

        let prev_check = match self.prev_check {
            Some(ref prev_check) => prev_check,
            None => return,
        };
        let dt = prev_check.time.elapsed();
        let d_user = timers.total_user_time - prev_check.total_user_time;
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
}
