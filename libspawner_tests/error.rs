use crate::exe;
use spawner::driver::{run, CommandReport, Report};

fn ensure_error(report: CommandReport, error: &str) {
    let json = report.to_json();
    assert_eq!(json["SpawnerError"][0], error);
}

#[test]
fn test_controller_without_argv() {
    let err = run(&["--controller"]).unwrap_err();
    assert_eq!(err.to_string(), "Controller must have an argv");
}

#[test]
fn test_multiple_controllers() {
    let err = run(&[
        "--separator=@",
        "--controller",
        exe!("empty"),
        "--@",
        "--controller",
        exe!("empty"),
    ])
    .unwrap_err();
    assert_eq!(err.to_string(), "There can be at most one controller");
}

#[test]
fn test_invalid_stdin_index() {
    let err = run(&["--out=*10.stdin", exe!("empty")]).unwrap_err();
    assert_eq!(err.to_string(), "Stdin index '10' is out of range");
}

#[test]
fn test_invalid_stdout_index() {
    let err = run(&["--in=*10.stdout", exe!("empty")]).unwrap_err();
    assert_eq!(err.to_string(), "Stdout index '10' is out of range");
}

fn run_single_controller_cmd(cmd: &str) -> Report {
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("arg_printer"),
        cmd,
        "--@",
        "--in=*0.stdout",
        exe!("loop"),
    ])
    .unwrap()
}

#[test]
fn test_invalid_agent_index() {
    let report = run_single_controller_cmd("10W#\n");
    ensure_error(report.at(0), "Agent index '10' is out of range\n");
}

#[test]
fn test_invalid_controller_command() {
    let report = run_single_controller_cmd("10WWW#\n");
    ensure_error(
        report.at(0),
        "Invalid controller command 'WWW' in '10WWW'\n",
    );
}

#[test]
fn test_invalid_controller_command_2() {
    let report = run_single_controller_cmd("A\n");
    ensure_error(report.at(0), "Missing '#' in controller message\n");
}
