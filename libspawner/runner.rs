use command::Command;
use process::Statistics;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Weak;

#[derive(Clone)]
pub enum TerminationReason {
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    Other,
}

#[derive(Clone)]
pub enum ExitStatus {
    Normal(i32),
    Terminated(TerminationReason),
}

#[derive(Clone)]
pub struct Report {
    pub command: Command,
    pub statistics: Statistics,
    pub exit_status: ExitStatus,
}

#[derive(Clone)]
pub struct Runner {
    pub(crate) is_killed: Weak<AtomicBool>,
}

impl Runner {
    pub fn kill(&self) {
        if let Some(flag) = self.is_killed.upgrade() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}
