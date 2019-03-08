use crate::common::{read_all, write_all, TmpDir};
use crate::exe;

use spawner_driver::run;

#[test]
fn exclusive_read() {
    let tmp = TmpDir::new();
    let file = tmp.file("file.txt");
    let stdout = tmp.file("stdout.txt");

    write_all(&file, "data");
    run(&[
        "--separator=@",
        "-d=0.5",
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
        "-d=0.5",
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
