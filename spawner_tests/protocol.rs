use crate::assert_approx_eq;
use crate::common::{read_all, TmpDir, APP, TIME_ERR};
use crate::term_reason::{
    check_tr, ensure_idle_time_limit_exceeded, ensure_ok, ensure_user_time_limit_exceeded,
    ensure_wall_clock_time_limit_exceeded,
};

use spawner_driver::{run, Report, TerminateReason};

#[test]
fn spawn_suspended() {
    let r = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "sleep",
        "2",
        "--@",
        "-y=0.5",
        APP,
        "loop",
        "2",
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(&r[1]);
}

#[test]
fn resume_agent_on_controller_termination() {
    let r = run(&[
        "--separator=@",
        "--controller",
        APP,
        "--@",
        "-d=1",
        "-tl=0.5",
        APP,
        "loop",
        "2",
    ])
    .unwrap();
    ensure_user_time_limit_exceeded(&r[1]);
}

#[test]
fn agent_termination_message() {
    let tmp = TmpDir::new();
    let stderr = tmp.file("stderr.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--err={}", stderr).as_str(),
        APP,
        "1W#\n",
        "wake_controller",
        "--@",
        "--in=*0.stdout",
        APP,
    ])
    .unwrap();
    assert_eq!(b"1T#\n", read_all(stderr).as_bytes());
}

#[test]
fn message_to_agent() {
    let tmp = TmpDir::new();
    let stderr1 = tmp.file("stderr1.txt");
    let stderr2 = tmp.file("stderr2.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "1W#\n2W#\n2#message\n",
        "--@",
        format!("--err={}", stderr1).as_str(),
        "--in=*0.stdout",
        APP,
        "pipe_loop",
        "--@",
        format!("--err={}", stderr2).as_str(),
        "--in=*0.stdout",
        APP,
        "pipe_loop",
    ])
    .unwrap();
    assert_eq!("", read_all(stderr1));
    assert_eq!("message\n", read_all(stderr2));
}

#[test]
fn message_from_agent() {
    let tmp = TmpDir::new();
    let stderr = tmp.file("stderr.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--err={}", stderr).as_str(),
        APP,
        "1W#\n",
        "wake_controller",
        "--@",
        "--in=*0.stdout",
        "--out=*0.stdin",
        APP,
        "message\n",
    ])
    .unwrap();
    assert_eq!("1#message\n1T#\n", read_all(stderr));
}

#[test]
fn controller_message_concatenation() {
    let tmp = TmpDir::new();
    let stderr = tmp.file("stderr.txt");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "1W",
        "#\n",
        "1",
        "#",
        "me",
        "ssa",
        "ge",
        "\n",
        "--@",
        "--in=*0.stdout",
        format!("--err={}", stderr).as_str(),
        APP,
        "pipe_loop",
    ])
    .unwrap();
    assert_eq!("message\n", read_all(stderr));
}

#[test]
fn agent_message_concatenation() {
    let tmp = TmpDir::new();
    let stderr = tmp.file("stderr.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--err={}", stderr).as_str(),
        APP,
        "1W#\n",
        "wake_controller",
        "--@",
        "--in=*0.stdout",
        "--out=*0.stdin",
        APP,
        "me",
        "ssa",
        "ge",
        "\n",
    ])
    .unwrap();
    assert_eq!("1#message\n1T#\n", read_all(stderr));
}

pub fn ensure_terminated_by_controller(report: &Report) {
    check_tr(report, TerminateReason::TerminatedByController);
}

#[test]
fn agent_terminated_by_controller() {
    let r = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "1S#\n",
        "2S#\n",
        "--@",
        "--in=*0.stdout",
        APP,
        "loop",
        "2",
        "--@",
        "--in=*0.stdout",
        APP,
        "loop",
        "2",
    ])
    .unwrap();
    ensure_terminated_by_controller(&r[1]);
    ensure_terminated_by_controller(&r[2]);
}

#[test]
fn agent_suspended_after_write() {
    let r = run(&[
        "--separator=@",
        "-d=4",
        "--json",
        "--@",
        "--controller",
        APP,
        "1W#\n",
        "loop",
        "4",
        "--@",
        "-y=1",
        "--in=*0.stdout",
        "--out=*0.stdin",
        APP,
        "message\n",
        "loop",
        "4",
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(&r[1]);
}

#[test]
fn controller_deadline() {
    let r = run(&["-d=1", "--controller", APP, "loop", "2"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn controller_idle_time_limit() {
    let r = run(&["-y=1", "--controller", APP, "sleep", "2"]).unwrap();
    ensure_idle_time_limit_exceeded(&r[0]);
}

#[test]
fn agent_user_time_limit() {
    let r = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "1W#\n",
        "loop",
        "2",
        "--@",
        "--in=*0.stdout",
        "-tl=0.5",
        APP,
        "loop",
        "2",
    ])
    .unwrap();
    ensure_user_time_limit_exceeded(&r[1]);
}

#[test]
fn agent_idle_time_limit() {
    let r = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        "1W#\n",
        "loop",
        "2",
        "--@",
        "--in=*0.stdout",
        "-y=0.5",
        APP,
        "sleep",
        "2",
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(&r[1]);
}

fn agent_time_usage(sleep_kind: &str) -> Vec<Report> {
    run(&[
        "--separator=@",
        "-d=2.5",
        "--@",
        "--controller",
        "--in=*1.stdout",
        "--out=*1.stdin",
        APP,
        "1W#\n",
        "wake_controller",
        "--@",
        "-d=1.5",
        APP,
        sleep_kind,
        "1",
        "message\n",
        sleep_kind,
        "1",
    ])
    .unwrap()
}

#[test]
fn reset_agent_user_time_and_wall_clock_time_usage() {
    let r = agent_time_usage("loop");
    ensure_ok(&r[1]);
    assert_approx_eq!(r[1].result.time, 2.0, 0.3);
    assert_approx_eq!(r[1].result.wall_clock_time, 2.0, TIME_ERR);
}

#[test]
fn reset_agent_idle_time_and_wall_clock_time_usage() {
    let r = agent_time_usage("sleep");
    let idle_time_usage = r[1].result.wall_clock_time - r[1].result.time;
    ensure_ok(&r[1]);
    assert_approx_eq!(idle_time_usage, 2.0, 0.3);
    assert_approx_eq!(r[1].result.wall_clock_time, 2.0, TIME_ERR);
}

fn controller_time_usage(sleep_kind: &str) -> Vec<Report> {
    run(&[
        "--separator=@",
        "-d=2.5",
        "--@",
        "--controller",
        "--in=*1.stdout",
        "--out=*1.stdin",
        "-d=1.5",
        APP,
        sleep_kind,
        "1",
        "1W#\n",
        sleep_kind,
        "1",
        "--@",
        APP,
        "sleep",
        "2",
    ])
    .unwrap()
}

#[test]
fn reset_controller_user_time_and_wall_clock_time_usage() {
    let r = controller_time_usage("loop");
    ensure_ok(&r[0]);
    assert_approx_eq!(r[0].result.time, 2.0, 0.3);
    assert_approx_eq!(r[0].result.wall_clock_time, 2.0, TIME_ERR);
}

#[test]
fn reset_controller_idle_time_and_wall_clock_time_usage() {
    let r = controller_time_usage("sleep");
    let idle_time_usage = r[0].result.wall_clock_time - r[0].result.time;
    ensure_ok(&r[0]);
    assert_approx_eq!(idle_time_usage, 2.0, 0.3);
    assert_approx_eq!(r[0].result.wall_clock_time, 2.0, TIME_ERR);
}
