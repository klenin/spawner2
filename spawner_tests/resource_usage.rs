use crate::assert_approx_eq;
use crate::common::{TmpDir, APP, MEM_ERR, TIME_ERR};

use spawner_driver::run;

fn total_user_time(arg: &str) {
    let r = run(&[
        "--wait-for-children",
        arg,
        APP,
        "loop",
        "1.0",
        "exec_rest",
        APP,
        "loop",
        "1.0",
        "exec_rest",
        APP,
        "loop",
        "1.0",
    ])
    .unwrap();
    assert_approx_eq!(r[0].result.time, 3.0, TIME_ERR * 3.0);
}

#[test]
fn total_user_time_mi_1ms() {
    total_user_time("-mi=1ms");
}

#[test]
fn total_user_time_mi_1s() {
    total_user_time("-mi=1s");
}

fn total_idle_time(arg: &str) {
    let r = run(&[
        "--wait-for-children",
        arg,
        APP,
        "sleep",
        "1.0",
        "exec_rest",
        APP,
        "sleep",
        "1.0",
        "exec_rest",
        APP,
        "sleep",
        "1.0",
    ])
    .unwrap();
    let total_idle_time = r[0].result.wall_clock_time - r[0].result.time;
    assert_approx_eq!(total_idle_time, 3.0, TIME_ERR * 3.0);
}

#[test]
fn total_idle_time_mi_1ms() {
    total_idle_time("-mi=1ms");
}

#[test]
fn total_idle_time_mi_1s() {
    total_idle_time("-mi=1s");
}

fn total_processes_created(arg: &str) {
    let r = run(&[
        "--wait-for-children",
        arg,
        APP,
        "sleep",
        "0.1",
        "exec_rest",
        APP,
        "sleep",
        "0.1",
        "exec_rest",
        APP,
        "sleep",
        "0.1",
        "exec_rest",
        APP,
        "sleep",
        "0.1",
        "exec_rest",
        APP,
        "sleep",
        "0.1",
    ])
    .unwrap();
    assert_eq!(r[0].result.processes_created, 5);
}

#[test]
fn total_processes_created_mi_1ms() {
    total_processes_created("-mi=1ms");
}

#[test]
fn total_processes_created_mi_1s() {
    total_processes_created("-mi=1s");
}

fn total_bytes_written(arg: &str) {
    let tmp = TmpDir::new();
    let _10mb = (10 * 1024).to_string();
    let f1 = tmp.file("1.txt");
    let f2 = tmp.file("2.txt");
    let r = run(&[
        "--wait-for-children",
        arg,
        APP,
        "fwrite",
        &f1,
        &_10mb,
        "exec_rest",
        APP,
        "fwrite",
        &f2,
        &_10mb,
    ])
    .unwrap();
    assert_approx_eq!(r[0].result.bytes_written, 20 * 1024 * 1024, MEM_ERR);
}

#[test]
fn total_bytes_written_mi_1ms() {
    total_bytes_written("-mi=1ms");
}

#[test]
fn total_bytes_written_mi_1s() {
    total_bytes_written("-mi=1s");
}

fn memory_usage(arg: &str) {
    let r = run(&[
        "--wait-for-children",
        arg,
        APP,
        "alloc",
        "8",
        "exec_rest_and_sleep",
        APP,
        "alloc",
        "8",
        "exec_rest_and_sleep",
        APP,
        "alloc",
        "8",
    ])
    .unwrap();
    assert_approx_eq!(r[0].result.memory, 24 * 1024 * 1024, MEM_ERR * 2);
}

#[test]
fn memory_usage_mi_1ms() {
    memory_usage("-mi=1ms");
}

#[test]
fn memory_usage_mi_1s() {
    memory_usage("-mi=1s");
}
