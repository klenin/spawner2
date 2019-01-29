use crate::{assert_approx_eq, exe};
use common::TmpDir;
use spawner::driver::new::run;
use spawner::runner::{ExitStatus, TerminationReason};

const MEM_ERR: u64 = 2 * 1024 * 1024; // 2MB
const TIME_ERR: u32 = 5; // 5 ms
const WRITE_ERR: u64 = 2 * 1024 * 1024;

#[test]
fn test_mem_limit() {
    let reports = run(&["-ml=10", exe!("alloc"), "10"]).unwrap();
    assert_eq!(
        reports[0].exit_status,
        ExitStatus::Terminated(TerminationReason::MemoryLimitExceeded)
    );
    assert_approx_eq!(
        reports[0].statistics.peak_memory_used,
        10 * 1024 * 1024,
        MEM_ERR
    );
}

#[test]
fn test_user_time_limit() {
    let reports = run(&["-tl=0.2", exe!("loop")]).unwrap();
    assert_eq!(
        reports[0].exit_status,
        ExitStatus::Terminated(TerminationReason::UserTimeLimitExceeded)
    );
    assert_approx_eq!(
        reports[0].statistics.total_user_time.subsec_millis(),
        200,
        TIME_ERR
    );
}

#[test]
fn test_write_limit() {
    let tmp = TmpDir::new();
    let reports = run(&[
        "-wl=10",
        exe!("file_writer"),
        tmp.file("file.txt").as_str(),
        format!("{}", 20 * 1024).as_str(),
    ])
    .unwrap();
    assert_eq!(
        reports[0].exit_status,
        ExitStatus::Terminated(TerminationReason::WriteLimitExceeded)
    );
    assert_approx_eq!(
        reports[0].statistics.total_bytes_written,
        10 * 1024 * 1024,
        WRITE_ERR
    );
}

#[test]
fn test_process_limit() {
    let reports = run(&["-process-count=1", exe!("two_proc")]).unwrap();
    assert_eq!(
        reports[0].exit_status,
        ExitStatus::Terminated(TerminationReason::ProcessLimitExceeded)
    );
}
