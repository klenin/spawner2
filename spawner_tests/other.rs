use crate::term_reason::{ensure_ok, ensure_wall_clock_time_limit_exceeded};

use crate::common::APP;
#[cfg(windows)]
use crate::common::{read_all, write_all, TmpDir};

use spawner_driver::run;

#[cfg(windows)]
#[test]
fn exclusive_read() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    write_all(&file, "data");
    run(&[
        "--separator=@",
        "-d=2",
        "--@",
        format!("--in=*e:{}", file).as_str(),
        APP,
        "--@",
        APP,
        "try_write",
        file.as_str(),
        "123",
    ])
    .unwrap();
    assert_eq!("data", read_all(file));
}

#[cfg(windows)]
#[test]
fn exclusive_write() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    run(&[
        "--separator=@",
        "-d=2",
        "--@",
        format!("--out=*e:{}", file).as_str(),
        APP,
        "A",
        "--@",
        APP,
        "try_write",
        file.as_str(),
        "123",
    ])
    .unwrap();
    assert_eq!("A", read_all(file));
}

#[cfg(windows)]
#[test]
fn exclusive_write_2() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
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
        APP,
        "try_write",
        file.as_str(),
        "123",
    ])
    .unwrap();
    assert_eq!("AA", read_all(file));
}

#[cfg(windows)]
#[test]
fn exclusive_read_2() {
    let tmp = TmpDir::new();
    let stdin = tmp.file("stdin.txt");
    let stderr = tmp.file("stderr.txt");
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
        APP,
        "try_write",
        stdin.as_str(),
        "123",
    ])
    .unwrap();
    assert_eq!("AA", read_all(stderr));
}

#[test]
fn wait_for_children() {
    println!("{}", std::env::current_dir().unwrap().to_str().unwrap());
    let r = run(&[
        "--separator=@",
        "--wait-for-children",
        "-d=1",
        APP,
        "exec_rest",
        APP,
        "loop",
        "2",
    ])
    .unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[cfg(windows)]
#[test]
fn search_in_path_enabled() {
    let r = run(&["-c", "cmd.exe", "/C", "EXIT", "0"]).unwrap();
    ensure_ok(&r[0]);
}

#[cfg(windows)]
#[test]
fn search_in_path_disabled() {
    let r = run(&["cmd.exe", "/C", "EXIT", "0"]).unwrap();
    assert!(!r[0].spawner_error.is_empty());
}

#[cfg(unix)]
#[test]
fn search_in_path_enabled() {
    let r = run(&["-c", "sh", "-c", "exit"]).unwrap();
    ensure_ok(&r[0]);
}

#[cfg(unix)]
#[test]
fn search_in_path_disabled() {
    let r = run(&["sh", "-c", "exit"]).unwrap();
    assert!(!r[0].spawner_error.is_empty());
}
