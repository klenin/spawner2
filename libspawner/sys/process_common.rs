use std::ffi::OsString;
use std::time::Duration;

#[derive(Clone)]
pub struct StartupInfo {
    pub app: OsString,
    pub args: Vec<OsString>,
    pub cwd: OsString,
    pub display_gui: bool,
}

pub struct SummaryInfo {
    /// the total amount of user-mode execution time for all active processes,
    /// as well as all terminated processes
    pub total_user_time: Duration,
    /// the total amount of kernel-mode execution time for all active processes,
    /// as well as all terminated processes
    pub total_kernel_time: Duration,
    /// the peak memory usage of all active processes, in bytes
    pub peak_memory_used: u64,
    /// total bytes written by a process
    pub total_bytes_written: u64,
    /// the total number of processes created
    pub total_processes: u64,
}

pub enum ProcessTreeStatus {
    Alive(SummaryInfo),
    Finished(i32),
}

impl SummaryInfo {
    pub fn zeroed() -> Self {
        Self {
            total_user_time: Duration::from_nanos(0),
            total_kernel_time: Duration::from_nanos(0),
            peak_memory_used: 0,
            total_bytes_written: 0,
            total_processes: 0,
        }
    }
}
