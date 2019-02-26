use crate::exe;
use common::{read_all, TmpDir};
use spawner::driver::new::{run, CommandReport};

#[test]
fn test_termination_msg() {
    let tmp = TmpDir::new();
    let err = tmp.file("err.txt");
    run(&[
        "--separator=@",
        "--controller",
        format!("--err={}", err).as_str(),
        exe!("in2out"),
        "--@",
        exe!("empty"),
    ])
    .unwrap();
    assert_eq!(b"1T#\n", read_all(err).as_bytes());
}

#[test]
fn test_multiple_termination_msgs() {
    let tmp = TmpDir::new();
    let err = tmp.file("err.txt");
    run(&[
        "--separator=@",
        "--controller",
        format!("--err={}", err).as_str(),
        exe!("in2out"),
        "--@",
        exe!("empty"),
        "--@",
        exe!("empty"),
        "--@",
        exe!("empty"),
    ])
    .unwrap();

    let data = read_all(err);
    let mut results: Vec<&str> = data.lines().map(|line| line).collect();
    results.sort();

    let expected = [b"1T#", b"2T#", b"3T#"];
    for (result, expected) in results.iter().zip(expected.iter()) {
        assert_eq!(&result.as_bytes(), expected);
    }
}

#[test]
fn test_message_to_agent() {
    let tmp = TmpDir::new();
    let out1 = tmp.file("out1.txt");
    let out2 = tmp.file("out2.txt");
    run(&[
        "--separator=@",
        "--controller",
        exe!("stdout_writer"),
        "2#message\n",
        "1",
        "--@",
        "--in=*0.stdout",
        format!("--err={}", out1).as_str(),
        exe!("in2out"),
        "--@",
        "--in=*0.stdout",
        format!("--err={}", out2).as_str(),
        exe!("in2out"),
    ])
    .unwrap();
    assert_eq!("", read_all(out1));
    assert_eq!("message\n", read_all(out2));
}

#[test]
fn test_message_from_agent() {
    let tmp = TmpDir::new();
    let out1 = tmp.file("out1.txt");
    let out2 = tmp.file("out2.txt");
    run(&[
        "--separator=@",
        "--controller",
        exe!("empty"),
        "--@",
        format!("--out={}", out1).as_str(),
        exe!("stdout_writer"),
        "message1\n",
        "1",
        "--@",
        format!("--out={}", out2).as_str(),
        exe!("stdout_writer"),
        "message2\n",
        "1",
    ])
    .unwrap();
    assert_eq!("1#message1\n", read_all(out1));
    assert_eq!("2#message2\n", read_all(out2));
}

pub fn ensure_terminated_by_controller(report: CommandReport) {
    let json = report.to_json();
    assert_eq!(json["TerminateReason"], "TerminatedByController");
}

#[test]
fn test_terminate_agent() {
    let report = run(&[
        "--separator=@",
        "-d=1",
        "--@",
        "--controller",
        exe!("arg_printer"),
        "1S#",
        "2S#",
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
