use crate::common::{read_all, write_all, TmpDir};
use crate::exe;
use crate::term_reason::ensure_wall_clock_time_limit_exceeded;

use spawner_driver::run;

#[test]
fn exclusive_read() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    write_all(&file, "data");
    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        format!("--in=*e:{}", file).as_str(),
        exe!("loop"),
        "--@",
        format!("--out={}", stdout).as_str(),
        exe!("open_file"),
        file.as_str(),
    ])
    .unwrap();

    assert_eq!("err", read_all(stdout));
}

#[test]
fn exclusive_write() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    run(&[
        "--separator=@",
        "-d=1",
        "--@",
        format!("--out=*e:{}", file).as_str(),
        exe!("loop"),
        "--@",
        format!("--out={}", stdout).as_str(),
        exe!("open_file"),
        file.as_str(),
    ])
    .unwrap();

    assert_eq!("err", read_all(stdout));
}

#[test]
fn wait_for_child_process() {
    let r = run(&["--separator=@", "-d=1", exe!("proc_spawner"), exe!("loop")]).unwrap();
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}
