use std::time::Duration;
use sys::pipe::{ReadPipe, WritePipe};

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

#[derive(Copy, Clone, Debug)]
pub struct ProcessInfo {
    /// The amount of time elapsed since process creation.
    pub wall_clock_time: Duration,
    /// The total amount of user-mode execution time for all active processes,
    /// as well as all terminated processes.
    pub total_user_time: Duration,
    /// The total amount of kernel-mode execution time for all active processes,
    /// as well as all terminated processes.
    pub total_kernel_time: Duration,
    /// The peak memory usage of all active processes, in bytes.
    pub peak_memory_used: u64,
    /// Total bytes written by a process.
    pub total_bytes_written: u64,
    /// The total number of processes created.
    pub total_processes: usize,
}

pub enum ProcessStatus {
    Running,
    Finished(u32),
    Crashed(u32, &'static str),
}

impl ProcessInfo {
    pub fn zeroed() -> Self {
        Self {
            wall_clock_time: Duration::from_nanos(0),
            total_user_time: Duration::from_nanos(0),
            total_kernel_time: Duration::from_nanos(0),
            peak_memory_used: 0,
            total_bytes_written: 0,
            total_processes: 0,
        }
    }
}
