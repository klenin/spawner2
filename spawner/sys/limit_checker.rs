use crate::process::{LimitViolation, ResourceLimits, ResourceUsage};

use std::time::{Duration, Instant};

struct PrevCheck {
    time: Instant,
    total_user_time: Duration,
}

pub struct LimitChecker {
    limits: ResourceLimits,
    prev_check: Option<PrevCheck>,
    wall_clock_time_zero: Duration,
    user_time_zero: Duration,
    total_idle_time: Duration,
}

impl LimitChecker {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits: limits,
            prev_check: None,
            wall_clock_time_zero: Duration::from_millis(0),
            user_time_zero: Duration::from_millis(0),
            total_idle_time: Duration::from_millis(0),
        }
    }

    pub fn reset_timers(&mut self, wall_clock_time_zero: Duration, user_time_zero: Duration) {
        self.wall_clock_time_zero = wall_clock_time_zero;
        self.user_time_zero = user_time_zero;
        self.total_idle_time = Duration::from_millis(0);
        self.prev_check = None;
    }

    pub fn check(&mut self, mut usage: ResourceUsage) -> Option<LimitViolation> {
        usage.wall_clock_time -= self.wall_clock_time_zero;
        usage.total_user_time -= self.user_time_zero;

        if let Some(ref prev_check) = self.prev_check {
            let dt = prev_check.time.elapsed();
            let mut d_user = usage.total_user_time - prev_check.total_user_time;
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

        self.prev_check = Some(PrevCheck {
            time: Instant::now(),
            total_user_time: usage.total_user_time,
        });

        fn gr<T: PartialOrd>(stat: T, limit: Option<T>) -> bool {
            limit.is_some() && stat > limit.unwrap()
        }

        let limits = &self.limits;
        if gr(usage.wall_clock_time, limits.wall_clock_time) {
            Some(LimitViolation::WallClockTimeLimitExceeded)
        } else if gr(self.total_idle_time, limits.total_idle_time) {
            Some(LimitViolation::IdleTimeLimitExceeded)
        } else if gr(usage.total_user_time, limits.total_user_time) {
            Some(LimitViolation::UserTimeLimitExceeded)
        } else if gr(usage.total_bytes_written, limits.total_bytes_written) {
            Some(LimitViolation::WriteLimitExceeded)
        } else if gr(usage.peak_memory_used, limits.peak_memory_used) {
            Some(LimitViolation::MemoryLimitExceeded)
        } else if gr(
            usage.total_processes_created,
            limits.total_processes_created,
        ) {
            Some(LimitViolation::ProcessLimitExceeded)
        } else if gr(usage.active_processes, limits.active_processes) {
            Some(LimitViolation::ActiveProcessLimitExceeded)
        } else {
            None
        }
    }
}
