use crate::common::TmpDir;
use crate::{assert_approx_eq, exe};

use spawner_driver::{run, Report, TerminateReason};

use std::u64;

const MEM_ERR: u64 = 2 * 1024 * 1024; // 2MB
const TIME_ERR: f64 = 0.050; // 50 ms

pub fn check_tr(report: &Report, tr: TerminateReason) {
    assert!(report.spawner_error.is_empty());
    assert_eq!(report.terminate_reason, tr);
}

pub fn ensure_mem_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::MemoryLimitExceeded);
    assert_approx_eq!(report.limit.memory.unwrap(), report.result.memory, MEM_ERR);
}

pub fn ensure_user_time_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::TimeLimitExceeded);
    assert_approx_eq!(report.limit.time.unwrap(), report.result.time, TIME_ERR);
}

pub fn ensure_write_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::WriteLimitExceeded);
    assert!(report.result.bytes_written >= report.limit.io_bytes.unwrap());
}

pub fn ensure_process_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::ProcessesCountLimitExceeded);
}

pub fn ensure_idle_time_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::IdleTimeLimitExceeded);
}

pub fn ensure_wall_clock_time_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::TimeLimitExceeded);
    assert_approx_eq!(
        report.limit.wall_clock_time.unwrap(),
        report.result.wall_clock_time,
        TIME_ERR
    );
}

pub fn ensure_abnormal_exit(report: &Report) {
    check_tr(report, TerminateReason::AbnormalExitProcess);
}

#[test]
fn mem_limit() {
    let r = run(&["-ml=10", exe!("alloc"), "10"]).unwrap();
    ensure_mem_limit_exceeded(&r[0]);
}

#[test]
fn user_time_limit() {
    let r = run(&["-tl=0.2", exe!("loop")]).unwrap();
    ensure_user_time_limit_exceeded(&r[0]);
}

#[test]
fn write_limit() {
    let tmp = TmpDir::new();
    let r = run(&[
        "-wl=10",
        exe!("file_writer"),
        tmp.file("file.txt").as_str(),
        format!("{}", 20 * 1024).as_str(),
    ])
    .unwrap();
    ensure_write_limit_exceeded(&r[0]);
}

#[test]
fn null_stdout_write_limit() {
    let r = run(&[
        "--out=*null",
        "-wl=8",
        exe!("stdout_writer"),
        "A",
        format!("{}", 10 * 1024 * 1024).as_str(),
    ])
    .unwrap();
    ensure_write_limit_exceeded(&r[0]);
}

#[test]
fn process_limit() {
    let r = run(&["-process-count=1", exe!("two_proc")]).unwrap();
    ensure_process_limit_exceeded(&r[0]);
}

#[test]
fn idle_time_limit() {
    let r = run(&["-y=0.2", exe!("sleep"), "1"]).unwrap();
    ensure_idle_time_limit_exceeded(&r[0]);
}

#[test]
fn wall_clock_time_limit_using_sleep() {
    let r = run(&["-d=0.2", exe!("sleep"), "1"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn wall_clock_time_limit_using_loop() {
    let r = run(&["-d=0.2", exe!("loop")]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn abnormal_exit() {
    let r = run(&["-d=1", exe!("abnormal_exit")]).unwrap();
    ensure_abnormal_exit(&r[0]);
}

#[test]
fn close_stdout_on_exit() {
    // if stdout_writer does not close stdout on exit then the reader will hang on stdin().read(...).
    let reports = run(&[
        "-d=1",
        "--separator=@",
        "--@",
        exe!("stdout_writer"),
        "AAA",
        "1000",
        "--@",
        "--in=*0.stdout",
        exe!("in2out"),
    ])
    .unwrap();
    for r in reports.iter() {
        check_tr(r, TerminateReason::ExitProcess);
        assert_eq!(r.exit_code, 0);
    }
}

#[test]
fn close_stdout_on_exit_2() {
    let reports = run(&[
        "-d=1",
        "--separator=@",
        "--@",
        exe!("stdout_writer"),
        "AAA",
        "1000",
        "--@",
        exe!("stdout_writer"),
        "AAA",
        "1000",
        "--@",
        "--in=*0.stdout",
        "--in=*1.stdout",
        exe!("in2out"),
    ])
    .unwrap();
    for r in reports.iter() {
        check_tr(r, TerminateReason::ExitProcess);
        assert_eq!(r.exit_code, 0);
    }
}
