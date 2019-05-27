use crate::term_reason::ensure_wall_clock_time_limit_exceeded;

use crate::common::APP;
#[cfg(windows)]
use crate::common::{read_all, write_all, TmpDir};

use spawner_driver::run;

#[cfg(windows)]
#[test]
fn exclusive_read() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    write_all(&file, "data");
    run(&[
        "--separator=@",
        "-d=2",
        "--@",
        format!("--in=*e:{}", file).as_str(),
        APP,
        "--@",
        format!("--out={}", stdout).as_str(),
        APP,
        "try_open",
        file.as_str(),
    ])
    .unwrap();

    assert_eq!("err", read_all(stdout));
}

#[cfg(windows)]
#[test]
fn exclusive_write() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    run(&[
        "--separator=@",
        "-d=2",
        "--@",
        format!("--out=*e:{}", file).as_str(),
        APP,
        "--@",
        format!("--out={}", stdout).as_str(),
        APP,
        "try_open",
        file.as_str(),
    ])
    .unwrap();

    assert_eq!("err", read_all(stdout));
}

#[cfg(windows)]
#[test]
fn exclusive_write_2() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        format!("--out=*e:{}", file).as_str(),
        APP,
        "A",
        "--@",
        format!("--out=*e:{}", file).as_str(),
        APP,
        "A",
        "--@",
        format!("--out={}", stdout).as_str(),
        APP,
        "try_open",
        file.as_str(),
    ])
    .unwrap();

    assert_eq!("AA", read_all(file));
    assert_eq!("err", read_all(stdout));
}

#[cfg(windows)]
#[test]
fn exclusive_read_2() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr = tmp.file("stderr.txt");
    let stdout = tmp.file("stdout.txt");

    write_all(&stdin, "A");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        format!("--in=*e:{}", stdin).as_str(),
        format!("--err={}", stderr).as_str(),
        APP,
        "pipe_loop",
        "--@",
        format!("--in=*e:{}", stdin).as_str(),
        format!("--err={}", stderr).as_str(),
        APP,
        "pipe_loop",
        "--@",
        format!("--out={}", stdout).as_str(),
        APP,
        "try_open",
        stdin.as_str(),
    ])
    .unwrap();

    assert_eq!("AA", read_all(stderr));
    assert_eq!("err", read_all(stdout));
}

#[test]
fn wait_for_child_process() {
    let r = run(&["--separator=@", "-d=1", APP, "exec_rest", APP, "loop", "2"]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}
