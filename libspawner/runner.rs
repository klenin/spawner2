use command::Command;
use process::Statistics;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Weak;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum TerminationReason {
    WallClockTimeLimitExceeded,
    IdleTimeLimitExceeded,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessLimitExceeded,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExitStatus {
    Finished(i32),
    Terminated(TerminationReason),
}

#[derive(Clone)]
pub struct RunnerReport {
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

impl ToString for RunnerReport {
    fn to_string(&self) -> String {
        let labels = [
            "app",
            "total user time",
            "total kernel time",
            "peak memory used",
            "total bytes written",
            "total processes",
            "exit status",
        ];
        let values = [
            self.command.app.clone(),
            dur_to_str(&self.statistics.total_user_time),
            dur_to_str(&self.statistics.total_kernel_time),
            self.statistics.peak_memory_used.to_string(),
            self.statistics.total_bytes_written.to_string(),
            self.statistics.total_processes.to_string(),
            estatus_to_str(&self.exit_status),
        ];
        let max_label_len = labels.iter().map(|x| x.len()).max().unwrap();

        let mut result = String::new();
        for (label, val) in labels.iter().zip(values.iter()) {
            write!(
                &mut result,
                "{}:{}{}\n",
                label,
                String::from(" ").repeat(2 + max_label_len - label.len()),
                val
            )
            .unwrap();
        }

        result
    }
}

fn dur_to_str(d: &Duration) -> String {
    let ms = d.as_secs() * 1000 + d.subsec_millis() as u64;
    (ms as f64 / 1000.0).to_string()
}

fn estatus_to_str(s: &ExitStatus) -> String {
    match s {
        ExitStatus::Finished(c) => format!("finished, exit code {}", c),
        ExitStatus::Terminated(r) => format!(
            "terminated, {}",
            match r {
                TerminationReason::WallClockTimeLimitExceeded => "wall clock time limit exceeded",
                TerminationReason::IdleTimeLimitExceeded => "idle time limit exceeded",
                TerminationReason::UserTimeLimitExceeded => "user time limit exceeded",
                TerminationReason::WriteLimitExceeded => "write limit exceeded",
                TerminationReason::MemoryLimitExceeded => "memory limit exceeded",
                TerminationReason::ProcessLimitExceeded => "process limit exceeded",
                TerminationReason::Other => "other",
            }
        ),
    }
}
