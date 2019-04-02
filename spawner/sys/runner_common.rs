use crate::command::Limits;
use crate::runner::{Statistics, TerminationReason};

use std::time::{Duration, Instant};

pub struct LimitChecker {
    limits: Limits,
    stats: Statistics,
    last_check_time: Option<Instant>,
    total_idle_time: Duration,
}

impl LimitChecker {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits: limits,
            stats: Statistics::zeroed(),
            last_check_time: None,
            total_idle_time: Duration::from_millis(0),
        }
    }

    pub fn stats(&self) -> Statistics {
        self.stats
    }

    pub fn reset_timers(&mut self) {
        self.stats.wall_clock_time = Duration::from_millis(0);
        self.stats.total_user_time = Duration::from_millis(0);
        self.total_idle_time = Duration::from_millis(0);
    }

    pub fn check(&mut self, new_stats: Statistics) -> Option<TerminationReason> {
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

        let limits = &self.limits;
        if gr(self.stats.wall_clock_time, limits.max_wall_clock_time) {
            Some(TerminationReason::WallClockTimeLimitExceeded)
        } else if gr(self.total_idle_time, limits.max_idle_time) {
            Some(TerminationReason::IdleTimeLimitExceeded)
        } else if gr(self.stats.total_user_time, limits.max_user_time) {
            Some(TerminationReason::UserTimeLimitExceeded)
        } else if gr(self.stats.total_bytes_written, limits.max_output_size) {
            Some(TerminationReason::WriteLimitExceeded)
        } else if gr(self.stats.peak_memory_used, limits.max_memory_usage) {
            Some(TerminationReason::MemoryLimitExceeded)
        } else if gr(self.stats.total_processes_created, limits.max_processes) {
            Some(TerminationReason::ProcessLimitExceeded)
        } else {
            None
        }
    }
}
