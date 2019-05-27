use crate::common::{read_all, write_all, TmpDir, APP};
use crate::term_reason::{
    ensure_idle_time_limit_exceeded, ensure_mem_limit_exceeded, ensure_user_time_limit_exceeded,
    ensure_wall_clock_time_limit_exceeded, ensure_write_limit_exceeded,
};

use spawner_driver::{run, Report};

use std::env;

struct Env {
    data: String,
}

impl Env {
    fn new() -> Env {
        let empty_argv: [&'static str; 0] = [];
        Env::with_argv(&empty_argv)
    }

    fn with_argv<T, U>(argv: T) -> Env
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let tmp = TmpDir::new();
        let out = tmp.file("out.txt");
        let mut args = vec![format!("--out={}", out)];
        args.extend(argv.into_iter().map(|s| s.as_ref().to_string()));
        args.push(APP.to_string());
        args.push("print_env".to_string());
        run(args).unwrap();
        Env {
            data: read_all(out),
        }
    }

    fn vars<'a>(&'a self) -> Vec<(&'a str, &'a str)> {
        self.data
            .lines()
            .map(|line| {
                let eq_pos = line.find('=').unwrap();
                (&line[..eq_pos], &line[eq_pos + 1..])
            })
            .collect()
    }
}

#[test]
fn clear_env() {
    let env = Env::with_argv(&["-env=clear"]);
    assert_eq!(env.vars(), Vec::new());
}

#[test]
fn define_var() {
    let env = Env::with_argv(&["-env=clear", "-D:NAME=VAR"]);
    assert_eq!(env.vars(), vec![("NAME", "VAR")]);
}

#[test]
fn define_var_2() {
    let env = Env::with_argv(&["-env=clear", "-D:A=B", "-D:C=D"]);
    let mut vars = env.vars();
    vars.sort_by(|a, b| a.0.partial_cmp(b.0).unwrap());
    assert_eq!(vars, vec![("A", "B"), ("C", "D")]);
}

#[test]
fn overwrite_var() {
    let env = Env::with_argv(&["-env=clear", "-D:NAME=VAR", "-D:NAME=VAR1"]);
    assert_eq!(env.vars(), vec![("NAME", "VAR1")]);
}

#[test]
fn default_env() {
    let env = Env::new();
    assert!(env.vars().len() != 0);
}

fn run_with_env(key: &str, val: &str, argv: &[&str]) -> Vec<Report> {
    env::set_var(key, val);
    let r = run(argv).unwrap();
    env::remove_var(key);
    r
}

#[test]
fn sp_time_limit() {
    let r = run_with_env("SP_TIME_LIMIT", "0.5", &[APP, "loop", "1"]);
    ensure_user_time_limit_exceeded(&r[0]);
}

#[test]
fn sp_deadline() {
    let r = run_with_env("SP_DEADLINE", "0.5", &[APP, "sleep", "1"]);
    ensure_wall_clock_time_limit_exceeded(&r[0]);
}

#[test]
fn sp_idle_time_limit() {
    let r = run_with_env("SP_IDLE_TIME_LIMIT", "0.5", &[APP, "sleep", "1"]);
    ensure_idle_time_limit_exceeded(&r[0]);
}

#[test]
fn sp_memory_limit() {
    let r = run_with_env("SP_MEMORY_LIMIT", "5", &[APP, "alloc", "5", "sleep", "1"]);
    ensure_mem_limit_exceeded(&r[0]);
}

#[test]
fn sp_write_limit() {
    let r = run_with_env("SP_WRITE_LIMIT", "0", &[APP, "print_n", "AAAAA", "10000"]);
    ensure_write_limit_exceeded(&r[0]);
}

#[test]
fn sp_report_file() {
    let tmp = TmpDir::new();
    let report = tmp.file("report.txt");
    run_with_env("SP_REPORT_FILE", &report, &[APP]);
    assert!(read_all(report).len() != 0);
}

#[test]
fn sp_environment() {
    env::set_var("SP_ENVIRONMENT", "clear");
    let env = Env::new();
    env::remove_var("SP_ENVIRONMENT");
    assert_eq!(env.vars(), Vec::new());
}

#[test]
fn sp_input_output_error_file() {
    let tmp = TmpDir::new();
    let input = tmp.file("input.txt");
    let output = tmp.file("output.txt");
    let error = tmp.file("error.txt");

    write_all(&input, "AAA");
    env::set_var("SP_INPUT_FILE", &input);
    env::set_var("SP_OUTPUT_FILE", &output);
    env::set_var("SP_ERROR_FILE", &error);

    let _ = run(&[APP, "pipe_loop"]);
    env::remove_var("SP_INPUT_FILE");
    env::remove_var("SP_OUTPUT_FILE");
    env::remove_var("SP_ERROR_FILE");

    assert_eq!(read_all(output), "AAA");
    assert_eq!(read_all(error), "AAA");
}
