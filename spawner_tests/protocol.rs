use crate::common::{read_all, write_all, TmpDir};
use crate::exe;
use crate::term_reason::{
    ensure_idle_time_limit_exceeded, ensure_user_time_limit_exceeded,
    ensure_wall_clock_time_limit_exceeded,
};

use spawner_driver::{run, CommandReport};

#[test]
fn spawn_suspended() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("sleep"),
        "2",
        "--@",
        "-y=0.5",
        exe!("loop"),
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(report.at(1));
}

#[test]
fn resume_agent_on_controller_termination() {
    let report = run(&[
        "--separator=@",
        "--controller",
        exe!("empty"),
        "--@",
        "-d=1",
        "-tl=0.5",
        exe!("loop"),
    ])
    .unwrap();
    ensure_user_time_limit_exceeded(report.at(1));
}

#[test]
fn resume_and_agent_termination_msgs() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr = tmp.file("stderr.txt");

    write_all(&stdin, "1W#\n");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--in={}", stdin).as_str(),
        format!("--err={}", stderr).as_str(),
        exe!("in2out"),
        "--@",
        "--in=*0.stdout",
        exe!("empty"),
    ])
    .unwrap();
    assert_eq!(b"1W#\n1T#\n", read_all(stderr).as_bytes());
}

#[test]
fn message_to_agent() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr1 = tmp.file("stderr1.txt");
    let stderr2 = tmp.file("stderr2.txt");

    write_all(&stdin, "1W#\n2W#\n2#message\n");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--in={}", stdin).as_str(),
        exe!("in2out"),
        "--@",
        format!("--err={}", stderr1).as_str(),
        "--in=*0.stdout",
        exe!("in2out"),
        "--@",
        format!("--err={}", stderr2).as_str(),
        "--in=*0.stdout",
        exe!("in2out"),
    ])
    .unwrap();
    assert_eq!("", read_all(stderr1));
    assert_eq!("message\n", read_all(stderr2));
}

#[test]
fn message_from_agent() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr = tmp.file("stderr.txt");

    write_all(&stdin, "1W#\n");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--in={}", stdin).as_str(),
        format!("--err={}", stderr).as_str(),
        exe!("in2out"),
        "--@",
        "--in=*0.stdout",
        "--out=*0.stdin",
        exe!("arg_printer"),
        "message\n",
    ])
    .unwrap();
    assert_eq!("1W#\n1#message\n1T#\n", read_all(stderr));
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
        exe!("arg_printer"),
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
        exe!("in2out"),
    ])
    .unwrap();
    assert_eq!("message\n", read_all(stderr));
}

#[test]
fn agent_message_concatenation() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr = tmp.file("stderr.txt");

    write_all(&stdin, "1W#\n");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        format!("--in={}", stdin).as_str(),
        format!("--err={}", stderr).as_str(),
        exe!("in2out"),
        "--@",
        "--in=*0.stdout",
        "--out=*0.stdin",
        exe!("arg_printer"),
        "me",
        "ssa",
        "ge",
        "\n",
    ])
    .unwrap();
    assert_eq!("1W#\n1#message\n1T#\n", read_all(stderr));
}

pub fn ensure_terminated_by_controller(report: CommandReport) {
    let json = report.to_json();
    assert_eq!(json["TerminateReason"], "TerminatedByController");
}

#[test]
fn agent_terminated_by_controller() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("arg_printer"),
        "1S#\n",
        "2S#\n",
        "--@",
        "--in=*0.stdout",
        exe!("loop"),
        "--@",
        "--in=*0.stdout",
        exe!("loop"),
    ])
    .unwrap();
    ensure_terminated_by_controller(report.at(1));
    ensure_terminated_by_controller(report.at(2));
}

#[test]
fn agent_suspended_after_write() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("loop"),
        "1W#\n",
        "--@",
        "-y=0.5",
        exe!("loop"),
        "message\n",
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(report.at(1));
}

#[test]
fn controller_suspended_after_write() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "-y=0.5",
        "--controller",
        exe!("loop"),
        "1message#\n",
        "--@",
        exe!("empty"),
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(report.at(0));
}

#[test]
fn controller_deadline() {
    let report = run(&["-d=1", "--controller", exe!("loop")]).unwrap();
    ensure_wall_clock_time_limit_exceeded(report.at(0));
}

#[test]
fn controller_idle_time_limit() {
    let report = run(&["-y=1", "--controller", exe!("sleep"), "2"]).unwrap();
    ensure_idle_time_limit_exceeded(report.at(0));
}

#[test]
fn agent_user_time_limit() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("loop"),
        "1W#\n",
        "--@",
        "--in=*0.stdout",
        "-tl=0.5",
        exe!("loop"),
    ])
    .unwrap();
    ensure_user_time_limit_exceeded(report.at(1));
}

#[test]
fn agent_idle_time_limit() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("loop"),
        "1W#\n",
        "--@",
        "--in=*0.stdout",
        "-y=0.5",
        exe!("sleep"),
        "2",
    ])
    .unwrap();
    ensure_idle_time_limit_exceeded(report.at(1));
}
