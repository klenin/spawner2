use spawner::driver::new::run;
use spawner::runner::{ExitStatus, TerminationReason};
use std::ops::{Add, Sub};

macro_rules! test_file {
    ($s:expr) => {
        concat!("../target/debug/", $s)
    };
}

macro_rules! exe {
    ($s:expr) => {
        concat!("../target/debug/", $s, ".exe")
    };
}

fn approx_eq<T>(a: T, b: T, diff: T) -> bool
where
    T: Add<Output = T> + Sub<Output = T> + PartialOrd + Copy,
{
    (a > (b - diff)) && (a < (b + diff))
}

macro_rules! assert_approx_eq {
    ($a:expr, $b:expr, $diff:expr) => {
        assert!(approx_eq($a, $b, $diff))
    };
}

const MEM_ERR: u64 = 2 * 1024 * 1024; // 2MB
const TIME_ERR: u32 = 5; // 5 ms
const WRITE_ERR: u64 = 512 * 1024; // 0.5MB

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
    let reports = run(&[
        "-wl=9",
        exe!("file_writer"),
        test_file!("test_write_limit.txt"),
        "10240",
    ])
    .unwrap();
    assert_eq!(
        reports[0].exit_status,
        ExitStatus::Terminated(TerminationReason::WriteLimitExceeded)
    );
    assert_approx_eq!(
        reports[0].statistics.total_bytes_written,
        9 * 1024 * 1024,
        WRITE_ERR
    );
}
