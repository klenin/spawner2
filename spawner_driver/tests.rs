use crate::cmd::*;
use crate::value_parser::StdinRedirectParser;

use spawner_opts::{CmdLineOptions, OptionValueParser};

use std::time::Duration;

fn fsec2dur(s: f64) -> Duration {
    Duration::from_micros((s * 1e6) as u64)
}

macro_rules! check_opt {
    ($argv:expr, $field:ident, $value:expr) => {{
        let mut cmd = Command::default();
        let _ = cmd.parse_argv($argv);
        assert_eq!(cmd.$field, $value);
    }};
}

#[test]
fn parse_opt_delimeters() {
    check_opt!(&["-tl=10"], time_limit, Some(fsec2dur(10.0)));
    check_opt!(&["-tl:10"], time_limit, Some(fsec2dur(10.0)));
    check_opt!(&["-tl", "10"], time_limit, Some(fsec2dur(10.0)));
}

#[test]
fn parse_basic_opts() {
    check_opt!(&["-tl=10"], time_limit, Some(fsec2dur(10.0)));
    check_opt!(&["-d=10"], wall_clock_time_limit, Some(fsec2dur(10.0)));
    check_opt!(&["-ml=10"], memory_limit, Some(10.0));
    check_opt!(&["-wl=10"], write_limit, Some(10.0));
    check_opt!(&["-s=1"], secure, true);
    check_opt!(&["-y=10"], idle_time_limit, Some(fsec2dur(10.0)));
    check_opt!(&["-lr=10"], load_ratio, 10.0);
    check_opt!(&["-lr=10%"], load_ratio, 10.0);
    check_opt!(&["-sw=1"], show_window, true);
    check_opt!(&["--debug=1"], debug, true);
    check_opt!(&["-mi=0.1"], monitor_interval, fsec2dur(0.1));
    check_opt!(&["-wd=asd"], working_directory, Some(String::from("asd")));
    check_opt!(&["-hr=1"], hide_report, true);
    check_opt!(&["-ho=1"], hide_output, true);
    check_opt!(&["-runas=1"], delegated, true);
    check_opt!(&["--delegated=1"], delegated, true);
    check_opt!(&["-u=user"], username, Some(String::from("user")));
    check_opt!(&["-p=password"], password, Some(String::from("password")));
    check_opt!(&["-c"], use_syspath, true);
    check_opt!(&["--systempath"], use_syspath, true);
    check_opt!(&["-sr=file"], output_file, Some(String::from("file")));
    check_opt!(&["--separator=sep"], separator, Some(String::from("sep")));
    check_opt!(&["-process-count=10"], process_count, Some(10));
    check_opt!(
        &["-active-process-count=10"],
        active_process_count,
        Some(10)
    );
    check_opt!(
        &["-active-connection-count=10"],
        active_connection_count,
        Some(10)
    );
    check_opt!(&["--controller"], controller, true);
    check_opt!(&["-j"], use_json, true);
    check_opt!(&["--json"], use_json, true);
    check_opt!(&["--wait-for-children"], wait_for_children, true);
}

#[test]
fn parse_env_type() {
    let mut cmd = Command::default();
    let _ = cmd.parse_argv(&["-env=clear"]);
    match cmd.env {
        Environment::Clear => {}
        _ => unreachable!(),
    }

    let _ = cmd.parse_argv(&["-env=inherit"]);
    match cmd.env {
        Environment::Inherit => {}
        _ => unreachable!(),
    }

    let _ = cmd.parse_argv(&["-env=user-default"]);
    match cmd.env {
        Environment::UserDefault => {}
        _ => unreachable!(),
    }
}

#[test]
fn parse_env_var() {
    let mut cmd = Command::default();
    let _ = cmd.parse_argv(&["-D:a=b", "-D:c=d"]);
    let v0 = &cmd.env_vars[0];
    assert_eq!(v0.0, "a");
    assert_eq!(v0.1, "b");
    let v1 = &cmd.env_vars[1];
    assert_eq!(v1.0, "c");
    assert_eq!(v1.1, "d");
}

macro_rules! check_file_flags {
    ($init_flush:expr, $init_exclusive:expr, $input:expr, $expected_flush:expr, $expected_exclusive:expr) => {{
        let mut cmd = Command::default();
        cmd.stdout_redirect.default_flags.flush = $init_flush;
        cmd.stdout_redirect.default_flags.exclusive = $init_flush;
        let _ = cmd.parse_argv(&["-ff", $input]);
        assert_eq!(cmd.stdout_redirect.default_flags.flush, $expected_flush);
        assert_eq!(
            cmd.stdout_redirect.default_flags.exclusive,
            $expected_exclusive
        );
    }};
}

#[test]
fn parse_file_flags() {
    check_file_flags!(false, false, "f", true, false);
    check_file_flags!(false, false, "e", false, true);
    check_file_flags!(false, false, "fe", true, true);
    check_file_flags!(true, true, "-f", false, true);
    check_file_flags!(true, true, "-e", true, false);
    check_file_flags!(true, true, "-f-e", false, false);
}

macro_rules! check_mem_value {
    ($input:expr, $expected:expr) => {{
        let mut cmd = Command::default();
        let _ = cmd.parse_argv(&["-ml", $input]);
        assert_eq!(cmd.memory_limit, Some($expected));
    }};
}

#[test]
fn parse_mem_value() {
    check_mem_value!("0", 0.0);
    check_mem_value!("0.0", 0.0);
    check_mem_value!("10", 10.0);
    check_mem_value!("0.123456", 0.123456);
    check_mem_value!("0.000001", 0.000001);
}

#[test]
fn parse_mem_value_exp() {
    check_mem_value!("10e1", 100.0);
    check_mem_value!("10E1", 100.0);
    check_mem_value!("10e-1", 1.0);
    check_mem_value!("10E-1", 1.0);
}

#[test]
fn parse_mem_value_sign() {
    check_mem_value!("-10.0", -10.0);
    check_mem_value!("+10.0", 10.0);
}

#[test]
fn parse_mem_value_degree() {
    check_mem_value!("10d", 0.0);
    check_mem_value!("100c", 0.0);
    check_mem_value!("1000m", 0.0);
    check_mem_value!("1000000u", 0.0);
    check_mem_value!("1000000000n", 0.0);
    check_mem_value!("1000000000000p", 0.0);
    check_mem_value!("1000000000000000f", 0.0);
    check_mem_value!("10%", 0.0);

    check_mem_value!("1024k", 1.0);
    check_mem_value!("10M", 10.0);
    check_mem_value!("10G", 10240.0);
    check_mem_value!("10T", 10485760.0);
    check_mem_value!("10P", 10737418240.0);
}

#[test]
fn parse_mem_value_unit() {
    check_mem_value!("1048576B", 1.0);
    check_mem_value!("8388608b", 1.0);
}

#[test]
fn parse_mem_value_suffix() {
    check_mem_value!("1024kB", 1.0);
    check_mem_value!("8192kb", 1.0);
    check_mem_value!("1MB", 1.0);
    check_mem_value!("8Mb", 1.0);
    check_mem_value!("1TB", 1048576.0);
    check_mem_value!("8Tb", 1048576.0);
    check_mem_value!("1PB", 1073741824.0);
    check_mem_value!("8Pb", 1073741824.0);
}

macro_rules! check_time_value {
    ($input:expr, $expected:expr) => {{
        let mut cmd = Command::default();
        let _ = cmd.parse_argv(&["-tl", $input]);
        assert_eq!(cmd.time_limit, Some(fsec2dur($expected)));
    }};
}

#[test]
fn parse_time_value_degree() {
    check_time_value!("100c", 1.0);
    check_time_value!("1000m", 1.0);
    check_time_value!("1000000u", 1.0);
    check_time_value!("1000000000n", 1.0);
    check_time_value!("1000000000000p", 1.0);
    check_time_value!("1000000000000000f", 1.0);

    check_time_value!("25%", 0.25);
    check_time_value!("1k", 1e3);
    check_time_value!("1M", 1e6);
    check_time_value!("1G", 1e9);
    check_time_value!("1T", 1e12);
    check_time_value!("1P", 1e15);
}

#[test]
fn parse_time_value_suffix() {
    check_time_value!("3s", 3.0);
    check_time_value!("1h", 3600.0);
    check_time_value!("1dm", 6.0);
    check_time_value!("1cm", 0.6);
    check_time_value!("1mm", 0.06);
    check_time_value!("10um", 0.00060);
    check_time_value!("1nd", 0.000086);
    check_time_value!("1000pd", 0.000086);
    check_time_value!("1000000fd", 0.000086);
}

impl ToString for RedirectList {
    fn to_string(&self) -> String {
        match self.items.len() {
            0 => format!("*{}:", self.default_flags.to_string()),
            _ => self.items[0].to_string(),
        }
    }
}

macro_rules! check_redirect {
    ($default:expr, ($($input:expr),*), $expected:expr) => {{
        let mut cmd = Command::default();
        let _ = StdinRedirectParser::parse(&mut cmd.stdin_redirect, $default);
        let _ = cmd.parse_argv(&[$("-i", $input),*]);
        assert_eq!(cmd.stdin_redirect.to_string(), $expected);
    }};

    (($($input:expr),*), $expected:expr) => {{
        let mut cmd = Command::default();
        let _ = cmd.parse_argv(&[$("-i", $input),*]);
        assert_eq!(cmd.stdin_redirect.to_string(), $expected);
    }};
}

#[test]
fn parse_redirect_flags() {
    check_redirect!("*-f-e:", ("*f:"), "*f-e:");
    check_redirect!("*-f-e:", ("*e:"), "*-fe:");
    check_redirect!("*-f-e:", ("*fe:"), "*fe:");
    check_redirect!("*-f-e:", ("*fe:", "*-fe:"), "*-fe:");
    check_redirect!("*fe:", ("*:"), "*-f-e:");
}

#[test]
fn parse_file_redirect() {
    check_redirect!(("file"), "*-f-e:file");
    check_redirect!(("*fe:", "*:file"), "*fe:file");
    check_redirect!(("*fe:file"), "*fe:file");
    check_redirect!(("*fe:", "*:", "*:file"), "*-f-e:file");
}

#[test]
fn parse_basic_pipe_redirect() {
    check_redirect!(("*std"), "*f-e:std");
    check_redirect!(("*null"), "*f-e:null");
    check_redirect!(("*0.stdout"), "*f-e:0.stdout");
}

#[test]
fn parse_pipe_redirect() {
    check_redirect!(("*std"), "*f-e:std");
    check_redirect!(("*fe:", "*:std"), "*fe:std");
    check_redirect!(("*fe:std"), "*fe:std");
    check_redirect!(("*fe:", "*:", "*:std"), "*-f-e:std");
    check_redirect!(("*fe:", "*:", "*std"), "*f-e:std");
}
