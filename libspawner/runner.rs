use command::Command;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;
use std::u64;
use sys::{ProcessTree, ProcessTreeStatus, SummaryInfo};

#[derive(Copy, Clone)]
pub enum TerminationReason {
    None,
    UserTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    Other,
}

#[derive(Clone)]
pub struct Report {
    pub cmd: Command,
    pub termination_reason: TerminationReason,
    pub user_time: Duration,
    pub kernel_time: Duration,
    pub peak_memory_used: u64,
    pub processes_created: u64,
    pub exit_code: i32,
}

#[derive(Clone)]
pub struct Runner {
    is_killed: Weak<AtomicBool>,
}

pub(crate) struct WaitHandle {
    pstree: Arc<Mutex<ProcessTree>>,
    monitoring_thread: thread::JoinHandle<Report>,
    runner: Runner,
}

struct MonitoringLoop {
    pstree: Weak<Mutex<ProcessTree>>,
    cmd: Command,
    is_killed: Arc<AtomicBool>,
}

pub(crate) fn run(cmd: Command) -> io::Result<WaitHandle> {
    let pstree = Arc::new(Mutex::new(ProcessTree::spawn(&cmd.info)?));

    let monitoring_loop = MonitoringLoop::new(Arc::downgrade(&pstree), cmd);
    let is_killed = Arc::downgrade(&monitoring_loop.is_killed);

    let thread_entry = move || match MonitoringLoop::start(monitoring_loop) {
        Err(e) => panic!(e),
        Ok(r) => r,
    };

    match thread::Builder::new().spawn(thread_entry) {
        Ok(handle) => Ok(WaitHandle {
            pstree: pstree,
            monitoring_thread: handle,
            runner: Runner {
                is_killed: is_killed,
            },
        }),
        Err(e) => {
            kill_pstree(pstree)?;
            Err(e)
        }
    }
}

impl Runner {
    pub fn kill(&self) {
        if let Some(flag) = self.is_killed.upgrade() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}

impl WaitHandle {
    pub(crate) fn runner(&self) -> &Runner {
        &self.runner
    }

    pub(crate) fn wait(self) -> io::Result<Report> {
        let result = self.monitoring_thread.join().map_err(|e| {
            let err_ref = e.downcast_ref::<io::Error>().unwrap();
            io::Error::new(err_ref.kind(), err_ref.to_string())
        });
        kill_pstree(self.pstree)?;
        result
    }
}

impl MonitoringLoop {
    fn new(tree: Weak<Mutex<ProcessTree>>, cmd: Command) -> Self {
        Self {
            pstree: tree,
            cmd: cmd,
            is_killed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn termination_reason(&self, info: &SummaryInfo) -> io::Result<TerminationReason> {
        let limits = &self.cmd.limits;
        if info.total_processes > limits.max_processes {
            Ok(TerminationReason::Other)
        } else if info.total_user_time > limits.max_user_time {
            Ok(TerminationReason::UserTimeLimitExceeded)
        } else if info.total_bytes_written > limits.max_output_size {
            Ok(TerminationReason::WriteLimitExceeded)
        } else if info.peak_memory_used > limits.max_memory_usage {
            Ok(TerminationReason::MemoryLimitExceeded)
        } else {
            Ok(TerminationReason::None)
        }
    }

    fn start(self) -> io::Result<Report> {
        let mut summary_info = SummaryInfo::zeroed();
        let mut termination_reason = TerminationReason::None;
        let mut exit_code = 0;

        while !self.is_killed.load(Ordering::SeqCst) {
            if let Some(pstree) = self.pstree.upgrade() {
                match pstree.lock().unwrap().status()? {
                    ProcessTreeStatus::Alive(info) => {
                        summary_info = info;
                        termination_reason = self.termination_reason(&summary_info)?;
                        match termination_reason {
                            TerminationReason::None => {}
                            _ => break,
                        }
                    }
                    ProcessTreeStatus::Finished(c) => {
                        exit_code = c;
                        break;
                    }
                }
                thread::sleep(self.cmd.monitor_interval);
            } else {
                // runner is dropped
                break;
            }
        }

        Ok(Report {
            cmd: self.cmd,
            termination_reason: termination_reason,
            user_time: summary_info.total_user_time,
            kernel_time: summary_info.total_kernel_time,
            peak_memory_used: summary_info.peak_memory_used,
            processes_created: summary_info.total_processes,
            exit_code: exit_code,
        })
    }
}

// assuming there is only one strong reference left
fn kill_pstree(tree: Arc<Mutex<ProcessTree>>) -> io::Result<()> {
    assert!(Arc::strong_count(&tree) == 1);
    Arc::try_unwrap(tree).unwrap().into_inner().unwrap().kill()
}
