use crate::assert_approx_eq;
use crate::common::{TmpDir, APP, MEM_ERR, TIME_ERR};

use spawner_driver::{run, Report, TerminateReason};

pub fn check_tr(report: &Report, tr: TerminateReason) {
    assert!(report.spawner_error.is_empty());
    assert_eq!(report.terminate_reason, tr);
}

pub fn ensure_ok(report: &Report) {
    check_tr(report, TerminateReason::ExitProcess);
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

pub fn ensure_active_process_count_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::ActiveProcessesCountLimitExceeded);
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

pub fn ensure_active_connection_count_limit_exceeded(report: &Report) {
    check_tr(report, TerminateReason::ActiveConnectionCountLimitExceeded);
}

#[test]
fn mem_limit() {
    let r = run(&["-d=3", "-ml=10", APP, "alloc", "10"]).unwrap();
    ensure_mem_limit_exceeded(&r[0]);
}

#[test]
fn user_time_limit() {
    let r = run(&["-tl=0.2", APP, "loop", "1"]).unwrap();
    ensure_user_time_limit_exceeded(&r[0]);
}

#[test]
fn write_limit() {
    let tmp = TmpDir::new();
    let r = run(&[
        "-wl=10",
        APP,
        "fwrite",
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
        APP,
        "print_n",
        "AAAAAAAAAA",
        format!("{}", 1024 * 1024).as_str(),
    ])
    .unwrap();
    ensure_write_limit_exceeded(&r[0]);
}

#[test]
fn process_limit() {
    let r = run(&[
        "-process-count=1",
        APP,
        "exec_rest_and_sleep",
        APP,
        "sleep",
        "1",
    ])
    .unwrap();
    ensure_process_limit_exceeded(&r[0]);
}

#[test]
fn active_process_limit() {
    let r = run(&[
        "-active-process-count=1",
        APP,
        "exec_rest_and_sleep",
        APP,
        "sleep",
        "1",
    ])
    .unwrap();
    ensure_active_process_count_limit_exceeded(&r[0]);
}

#[test]
fn single_active_process() {
    let r = run(&["-active-process-count=1", APP, "sleep", "1"]).unwrap();
    ensure_ok(&r[0]);
}

#[test]
fn idle_time_limit() {
    let r = run(&["-y=0.2", APP, "sleep", "1"]).unwrap();
    ensure_idle_time_limit_exceeded(&r[0]);
}

#[test]
fn wall_clock_time_limit_using_sleep() {
    let r = run(&["-d=0.2", APP, "sleep", "1"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn wall_clock_time_limit_using_loop() {
    let r = run(&["-d=0.2", APP, "loop", "1"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn abnormal_exit() {
    let r = run(&["-d=2", APP, "abnormal_exit"]).unwrap();
    ensure_abnormal_exit(&r[0]);
}

#[test]
fn close_stdout_on_exit() {
    // if stdout writer does not close stdout on exit then the reader will hang on stdin().read(...).
    let reports = run(&[
        "-d=1",
        "--separator=@",
        "--@",
        APP,
        "print_n",
        "AAA",
        "1000",
        "--@",
        "--in=*0.stdout",
        APP,
        "pipe_loop",
    ])
    .unwrap();
    for r in reports.iter() {
        ensure_ok(r);
        assert_eq!(r.exit_code, 0);
    }
}

#[test]
fn close_stdout_on_exit_2() {
    let reports = run(&[
        "-d=1",
        "--separator=@",
        "--@",
        APP,
        "print_n",
        "AAA",
        "1000",
        "--@",
        APP,
        "print_n",
        "AAA",
        "1000",
        "--@",
        "--in=*0.stdout",
        "--in=*1.stdout",
        APP,
        "pipe_loop",
    ])
    .unwrap();
    for r in reports.iter() {
        ensure_ok(r);
        assert_eq!(r.exit_code, 0);
    }
}

fn exceed_connection_limit(create_sockets: &'static str) {
    let r = run(&["-active-connection-count=1", APP, create_sockets, "2"]).unwrap();
    ensure_active_connection_count_limit_exceeded(&r[0]);
}

#[test]
fn active_connections_limit_exceeded() {
    exceed_connection_limit("create_tcpv4_sockets");
    exceed_connection_limit("create_tcpv6_sockets");
    exceed_connection_limit("create_udpv4_sockets");
    exceed_connection_limit("create_udpv6_sockets");
}

fn connection_limit_ok(create_sockets: &'static str) {
    let r = run(&["-active-connection-count=1", APP, create_sockets, "1"]).unwrap();
    ensure_ok(&r[0]);
}

#[test]
fn active_connections_limit_ok() {
    connection_limit_ok("create_tcpv4_sockets");
    connection_limit_ok("create_tcpv6_sockets");
    connection_limit_ok("create_udpv4_sockets");
    connection_limit_ok("create_udpv6_sockets");
}
