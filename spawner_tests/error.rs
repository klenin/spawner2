use crate::common::APP;

use spawner_driver::{run, Report};

fn ensure_error(report: &Report, error: &str) {
    assert_eq!(report.spawner_error[0].to_string(), error);
}

#[test]
fn multiple_controllers() {
    let err = run(&[
        "--separator=@",
        "--controller",
        APP,
        "--@",
        "--controller",
        APP,
    ])
    .unwrap_err();
    assert_eq!(err.to_string(), "There can be at most one controller");
}

#[test]
fn invalid_stdin_index() {
    let err = run(&["--out=*10.stdin", APP]).unwrap_err();
    assert_eq!(err.to_string(), "Stdin index '10' is out of range");
}

#[test]
fn invalid_stdout_index() {
    let err = run(&["--in=*10.stdout", APP]).unwrap_err();
    assert_eq!(err.to_string(), "Stdout index '10' is out of range");
}

fn run_single_controller_cmd(cmd: &str) -> Vec<Report> {
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        APP,
        cmd,
        "--@",
        "--in=*0.stdout",
        APP,
        "loop",
        "2",
    ])
    .unwrap()
}

#[test]
fn invalid_agent_index() {
    let r = run_single_controller_cmd("10W#\n");
    ensure_error(&r[0], "Agent index '10' is out of range");
}

#[test]
fn invalid_controller_command() {
    let r = run_single_controller_cmd("10WWW#\n");
    ensure_error(&r[0], "Invalid controller command 'WWW' in '10WWW'");
}

#[test]
fn invalid_controller_command_2() {
    let r = run_single_controller_cmd("A\n");
    ensure_error(&r[0], "Missing '#' in controller message");
}
