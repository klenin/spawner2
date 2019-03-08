use crate::common::{read_all, write_all, TmpDir};
use crate::exe;

use spawner_driver::run;

#[test]
fn stdin_from_file() {
    let tmp = TmpDir::new();
    let input_data = "1".repeat(30);
    let input = tmp.file("in.txt");
    let output = tmp.file("out.txt");

    write_all(input.as_str(), &input_data);
    run(&[
        format!("--in={}", input).as_str(),
        format!("--out={}", output).as_str(),
        exe!("in2out"),
    ])
    .unwrap();
    let output_data = read_all(output);
    assert_eq!(input_data, output_data);
}

#[test]
fn stdin_from_2_files() {
    let tmp = TmpDir::new();
    let input_data = "1".repeat(60);
    let input_1 = tmp.file("in1.txt");
    let input_2 = tmp.file("in2.txt");
    let output = tmp.file("out.txt");

    write_all(input_1.as_str(), &input_data[..30]);
    write_all(input_2.as_str(), &input_data[30..]);
    run(&[
        format!("--in={}", input_1).as_str(),
        format!("--in={}", input_2).as_str(),
        format!("--out={}", output).as_str(),
        exe!("in2out"),
    ])
    .unwrap();
    assert_eq!(input_data, read_all(output));
}

#[test]
fn stdin_from_stdout() {
    let tmp = TmpDir::new();
    let output = tmp.file("out.txt");
    run(&[
        "--separator=@",
        "--in=*1.stdout",
        format!("--out={}", output).as_str(),
        exe!("in2out"),
        "--@",
        "--out=*0.stdin",
        exe!("stdout_writer"),
        "AAA",
        "20",
    ])
    .unwrap();
    let output_data = read_all(output);
    assert_eq!("AAA".repeat(20), output_data);
}

#[test]
fn stdin_from_2_stdouts() {
    let tmp = TmpDir::new();
    let output = tmp.file("out.txt");
    run(&[
        "--separator=@",
        "--in=*1.stdout",
        format!("--out={}", output).as_str(),
        exe!("in2out"),
        "--@",
        "--out=*0.stdin",
        exe!("stdout_writer"),
        "AAA",
        "10",
        "--@",
        "--out=*0.stdin",
        exe!("stdout_writer"),
        "AAA",
        "10",
    ])
    .unwrap();
    let output_data = read_all(output);
    assert_eq!("AAA".repeat(20), output_data);
}

#[test]
fn stdout_to_file() {
    let tmp = TmpDir::new();
    let output = tmp.file("out.txt");
    run(&[
        format!("--out={}", output).as_str(),
        exe!("stdout_writer"),
        "AAA",
        "20",
    ])
    .unwrap();
    assert_eq!("AAA".repeat(20), read_all(output));
}

#[test]
fn stdout_to_2_files() {
    let tmp = TmpDir::new();
    let output_1 = tmp.file("out1.txt");
    let output_2 = tmp.file("out2.txt");
    run(&[
        format!("--out={}", output_1).as_str(),
        format!("--out={}", output_2).as_str(),
        exe!("stdout_writer"),
        "AAA",
        "20",
    ])
    .unwrap();
    assert_eq!("AAA".repeat(20), read_all(output_1));
    assert_eq!("AAA".repeat(20), read_all(output_2));
}

#[test]
fn multiple_stdouts_to_multiple_stdins() {
    let tmp = TmpDir::new();
    let output_1 = tmp.file("out1.txt");
    let output_2 = tmp.file("out2.txt");
    run(&[
        "--separator=@",
        exe!("stdout_writer"),
        "A",
        "20",
        "--@",
        exe!("stdout_writer"),
        "A",
        "20",
        "--@",
        "--in=*0.stdout",
        "--in=*1.stdout",
        format!("--out={}", output_1).as_str(),
        exe!("in2out"),
        "--@",
        "--in=*0.stdout",
        "--in=*1.stdout",
        format!("--out={}", output_2).as_str(),
        exe!("in2out"),
    ])
    .unwrap();
    assert_eq!("A".repeat(40), read_all(output_1));
    assert_eq!("A".repeat(40), read_all(output_2));
}

#[test]
fn multiple_stdouts_to_file() {
    let tmp = TmpDir::new();
    let out = tmp.file("out.txt");
    run(&[
        "--separator=@",
        format!("--out={}", out).as_str(),
        "--@",
        exe!("stdout_writer"),
        "AAA",
        "20",
        "--@",
        exe!("stdout_writer"),
        "AAA",
        "20",
    ])
    .unwrap();
    assert_eq!("AAA".repeat(40), read_all(out));
}
