use crate::assert_approx_eq;
use crate::common::{APP, MEM_ERR, TIME_ERR};

use spawner_driver::run;

#[test]
fn total_user_time() {
    let r = run(&[
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
fn total_idle_time() {
    let r = run(&[
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
fn total_processes_created() {
    let r = run(&[
        APP,
        "sleep",
        "0",
        "exec_rest",
        APP,
        "sleep",
        "0.5",
        "exec_rest",
        APP,
        "sleep",
        "0",
        "exec_rest",
        APP,
        "sleep",
        "0.5",
        "exec_rest",
        APP,
        "sleep",
        "0",
    ])
    .unwrap();
    assert_eq!(r[0].result.processes_created, 5);
}

#[test]
fn total_bytes_written() {
    let r = run(&[
        APP,
        "A",
        "exec_rest",
        APP,
        "A",
        "exec_rest",
        APP,
        "A",
        "exec_rest",
        APP,
        "A",
        "exec_rest",
        APP,
        "A",
    ])
    .unwrap();
    assert_eq!(r[0].result.bytes_written, 5);
}

#[test]
fn memory_usage() {
    let r = run(&[
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
