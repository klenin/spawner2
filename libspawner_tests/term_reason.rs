use crate::{assert_approx_eq, assert_flt_eq, exe};
use common::TmpDir;
use spawner::driver::new::{run, CommandReport};
use spawner::runner::{ExitStatus, TerminationReason};
use std::time::Duration;
use std::u64;

const MEM_ERR: u64 = 2 * 1024 * 1024; // 2MB
const TIME_ERR: f64 = 0.005; // 5 ms
const WRITE_ERR: u64 = 2 * 1024 * 1024;

pub fn mb2b(mb: f64) -> u64 {
    let b = mb * 1024.0 * 1024.0;
    if b.is_infinite() {
        u64::MAX
    } else {
        b as u64
    }
}

fn dur2sec(d: &Duration) -> f64 {
    let us = d.as_secs() as f64 * 1e6 + d.subsec_micros() as f64;
    us / 1e6
}

fn ensure_mem_limit_exceeded(report: CommandReport) {
    let runner_report = report.runner_report.unwrap();
    let json = report.to_json();
    let mem_used = runner_report.process_info.peak_memory_used;
    let mem_limit_b = mb2b(report.cmd.memory_limit.unwrap());

    assert_eq!(
        runner_report.exit_status,
        ExitStatus::Terminated(TerminationReason::MemoryLimitExceeded)
    );
    assert_approx_eq!(mem_used, mem_limit_b, MEM_ERR);
    assert_eq!(json["Limit"]["Memory"], mem_limit_b);
    assert_eq!(json["Result"]["Memory"], mem_used);
    assert_eq!(json["TerminateReason"], "MemoryLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_user_time_limit_exceeded(report: CommandReport) {
    let runner_report = report.runner_report.unwrap();
    let json = report.to_json();
    let time_used = dur2sec(&runner_report.process_info.total_user_time);
    let time_limit = dur2sec(report.cmd.time_limit.as_ref().unwrap());

    assert_eq!(
        runner_report.exit_status,
        ExitStatus::Terminated(TerminationReason::UserTimeLimitExceeded)
    );
    assert_approx_eq!(time_used, time_limit, TIME_ERR);
    assert_flt_eq!(json["Limit"]["Time"].as_f64().unwrap(), time_limit);
    assert_flt_eq!(json["Result"]["Time"].as_f64().unwrap(), time_used);
    assert_eq!(json["TerminateReason"], "TimeLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_write_limit_exceeded(report: CommandReport) {
    let runner_report = report.runner_report.unwrap();
    let json = report.to_json();
    let bytes_written = runner_report.process_info.total_bytes_written;
    let write_limit = mb2b(report.cmd.write_limit.unwrap());

    assert_eq!(
        runner_report.exit_status,
        ExitStatus::Terminated(TerminationReason::WriteLimitExceeded)
    );
    assert_approx_eq!(bytes_written, write_limit, WRITE_ERR);
    assert_eq!(json["Limit"]["IOBytes"], write_limit);
    assert_eq!(json["Result"]["BytesWritten"], bytes_written);
    assert_eq!(json["TerminateReason"], "WriteLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_process_limit_exceeded(report: CommandReport) {
    let runner_report = report.runner_report.unwrap();
    let json = report.to_json();
    let total_processes = runner_report.process_info.total_processes;
    let process_limit = report.cmd.process_count.unwrap();

    assert_eq!(
        runner_report.exit_status,
        ExitStatus::Terminated(TerminationReason::ProcessLimitExceeded)
    );
    assert!(total_processes > process_limit);
    assert_eq!(json["TerminateReason"], "ProcessesCountLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_idle_time_limit_exceeded(report: CommandReport) {
    let json = report.to_json();
    let time_limit = dur2sec(report.cmd.idle_time_limit.as_ref().unwrap());

    assert_eq!(
        report.runner_report.unwrap().exit_status,
        ExitStatus::Terminated(TerminationReason::IdleTimeLimitExceeded)
    );
    assert_flt_eq!(json["Limit"]["IdlenessTime"].as_f64().unwrap(), time_limit);
    assert_eq!(json["TerminateReason"], "IdleTimeLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_wall_clock_time_limit_exceeded(report: CommandReport) {
    let runner_report = report.runner_report.unwrap();
    let json = report.to_json();
    let time_used = dur2sec(&runner_report.process_info.wall_clock_time);
    let time_limit = dur2sec(report.cmd.wall_clock_time_limit.as_ref().unwrap());

    assert_eq!(
        runner_report.exit_status,
        ExitStatus::Terminated(TerminationReason::WallClockTimeLimitExceeded)
    );
    assert_approx_eq!(time_used, time_limit, TIME_ERR);
    assert_flt_eq!(json["Limit"]["WallClockTime"].as_f64().unwrap(), time_limit);
    assert_flt_eq!(json["Result"]["WallClockTime"].as_f64().unwrap(), time_used);
    assert_eq!(json["TerminateReason"], "TimeLimitExceeded");
    assert_eq!(json["SpawnerError"][0], "<none>");
}

fn ensure_abnormal_exit(report: CommandReport) {
    let json = report.to_json();
    assert_eq!(json["TerminateReason"], "AbnormalExitProcess");
}

#[test]
fn test_mem_limit() {
    let report = run(&["-ml=10", exe!("alloc"), "10"]).unwrap();
    ensure_mem_limit_exceeded(report.at(0));
}

#[test]
fn test_user_time_limit() {
    let report = run(&["-tl=0.2", exe!("loop")]).unwrap();
    ensure_user_time_limit_exceeded(report.at(0));
}

#[test]
fn test_write_limit() {
    let tmp = TmpDir::new();
    let report = run(&[
        "-wl=10",
        exe!("file_writer"),
        tmp.file("file.txt").as_str(),
        format!("{}", 20 * 1024).as_str(),
    ])
    .unwrap();
    ensure_write_limit_exceeded(report.at(0));
}

#[test]
fn test_null_stdout_write_limit() {
    let report = run(&[
        "--out=*null",
        "-wl=8",
        exe!("stdout_writer"),
        "A",
        format!("{}", 10 * 1024 * 1024).as_str(),
    ])
    .unwrap();
    ensure_write_limit_exceeded(report.at(0));
}

#[test]
fn test_process_limit() {
    let report = run(&["-process-count=1", exe!("two_proc")]).unwrap();
    ensure_process_limit_exceeded(report.at(0));
}

#[test]
fn test_idle_time_limit() {
    let report = run(&["-y=0.2", exe!("sleep"), "1"]).unwrap();
    ensure_idle_time_limit_exceeded(report.at(0));
}

#[test]
fn test_wall_clock_time_limit_using_sleep() {
    let report = run(&["-d=0.2", exe!("sleep"), "1"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(report.at(0));
}

#[test]
fn test_wall_clock_time_limit_using_loop() {
    let report = run(&["-d=0.2", exe!("loop")]).unwrap();
    ensure_wall_clock_time_limit_exceeded(report.at(0));
}

#[test]
fn test_abnormal_exit() {
    let report = run(&["-d=0.2", exe!("abnormal_exit")]).unwrap();
    ensure_abnormal_exit(report.at(0));
}
