pub mod opts;
mod value_parser;

#[cfg(test)]
mod tests;

use crate::{Error, Result};
use command::{self, Command};
use driver::new::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use driver::prelude::*;
use json::{array, object, stringify_pretty, JsonValue};
use runner::{ExitStatus, Report, TerminationReason};
use session::{IstreamSrc, OstreamDst, Session, StdioMapping};
use std::env;
use std::fs;
use std::io::Write;
use std::time::Duration;
use std::u64;

pub struct Driver {
    pub controller: Option<usize>,
    pub cmds: Vec<Options>,
}

pub fn parse<T, U>(argv: T) -> Result<Driver>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let argv: Vec<String> = argv.into_iter().map(|x| x.as_ref().to_string()).collect();
    let mut default_opts = Options::default();
    let mut pos = 0;
    let mut cmds: Vec<Options> = Vec::new();
    let mut controller: Option<usize> = None;

    while pos < argv.len() {
        let mut opts = default_opts.clone();
        let num_opts = match opts.parse(&argv[pos..]) {
            Ok(n) => n,
            Err(s) => return Err(Error::from(s)),
        };
        pos += num_opts;

        let mut sep_pos = argv.len();
        if let Some(sep) = &opts.separator {
            let full_sep = format!("--{}", sep);
            if let Some(i) = argv[pos..].iter().position(|x| x == &full_sep) {
                sep_pos = pos + i;
            }
        }
        opts.argv.extend_from_slice(&argv[pos..sep_pos]);
        pos = sep_pos + 1;

        if opts.argv.is_empty() {
            if opts.controller {
                return Err(Error::from("controller must have argv"));
            }
            default_opts = opts;
        } else if opts.controller && controller.is_some() {
            return Err(Error::from("there can be at most one controller"));
        } else {
            if opts.controller {
                controller = Some(cmds.len());
            }
            default_opts.separator = opts.separator.clone();
            cmds.push(opts);
        }
    }

    Ok(Driver {
        controller: controller,
        cmds: cmds,
    })
}

pub fn run<T, U>(argv: T) -> Result<Vec<Report>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let driver = parse(argv)?;
    driver.run()
}

pub fn main() {
    let args: Vec<_> = env::args().collect();
    let driver = match parse(&args[1..]) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    if driver.cmds.len() == 0 {
        println!("{}", Options::help());
        return;
    }

    let reports = driver.run();
    let mut is_err_printed = false;
    for (idx, cmd) in driver.cmds.iter().enumerate() {
        let mut is_err = false;
        let report_str = if cmd.use_json {
            stringify_pretty(json_report(&reports, idx, cmd), 4)
        } else {
            match &reports {
                Ok(x) => x[idx].to_string(),
                Err(e) => {
                    is_err = true;
                    e.to_string()
                }
            }
        };

        let hide = cmd.hide_report || (is_err && is_err_printed);
        if !hide {
            is_err_printed = is_err;
            println!("{}", report_str);
        }

        if let Some(filename) = &cmd.output_file {
            let _ = fs::remove_file(filename);
            let _ =
                fs::File::create(filename).and_then(|mut file| write!(&mut file, "{}", report_str));
        }
    }
}

impl Driver {
    pub fn run(&self) -> Result<Vec<Report>> {
        if self.cmds.len() == 0 {
            return Ok(Vec::new());
        }

        let mut sess = Session::new();
        let stdio_mappings: Vec<StdioMapping> = self
            .cmds
            .iter()
            .map(|x| sess.add_cmd(Command::from(x)))
            .collect();

        for (opt, mapping) in self.cmds.iter().zip(stdio_mappings.iter()) {
            redirect_istream(
                &mut sess,
                mapping.stdin,
                &stdio_mappings,
                &opt.stdin_redirect,
            )?;
            redirect_ostream(
                &mut sess,
                mapping.stdout,
                &stdio_mappings,
                &opt.stdout_redirect,
            )?;
            redirect_ostream(
                &mut sess,
                mapping.stderr,
                &stdio_mappings,
                &opt.stderr_redirect,
            )?;
        }

        sess.spawn()?.wait()
    }
}

fn redirect_istream(
    sess: &mut Session,
    istream: usize,
    stdio_mappings: &Vec<StdioMapping>,
    redirect_list: &StdioRedirectList,
) -> Result<()> {
    for redirect in redirect_list.items.iter() {
        match &redirect.kind {
            StdioRedirectKind::File(s) => {
                sess.connect_istream(istream, IstreamSrc::File(s.as_str()))?;
            }
            StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                PipeKind::Null => { /* pipes are null by default */ }
                PipeKind::Std => { /* todo */ }
                PipeKind::Stdout(i) => {
                    // check i
                    sess.connect_istream(istream, IstreamSrc::Ostream(stdio_mappings[*i].stdout))?;
                }
                _ => {}
            },
        }
    }
    Ok(())
}

fn redirect_ostream(
    sess: &mut Session,
    ostream: usize,
    stdio_mappings: &Vec<StdioMapping>,
    redirect_list: &StdioRedirectList,
) -> Result<()> {
    for redirect in redirect_list.items.iter() {
        match &redirect.kind {
            StdioRedirectKind::File(s) => {
                sess.connect_ostream(ostream, OstreamDst::File(s.as_str()))?;
            }
            StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                PipeKind::Null => { /* pipes are null by default */ }
                PipeKind::Std => { /* todo */ }
                PipeKind::Stdin(i) => {
                    // check i
                    sess.connect_ostream(ostream, OstreamDst::Istream(stdio_mappings[*i].stdin))?;
                }
                PipeKind::Stderr(_) => {
                    // todo: c++ spawner can redirect stderr to other stderr
                }
                _ => {}
            },
        }
    }
    Ok(())
}

fn mb2b(mb: f64) -> u64 {
    if mb.is_infinite() {
        u64::MAX
    } else {
        (mb * 1024.0 * 1024.0) as u64
    }
}

impl From<&Options> for Command {
    fn from(opts: &Options) -> Command {
        command::Builder::new(opts.argv[0].clone())
            .args(opts.argv.iter().skip(1))
            .current_dir(match opts.working_directory {
                Some(ref d) => d.clone(),
                None => String::new(),
            })
            .max_wall_clock_time(opts.wall_clock_time_limit)
            .max_idle_time(opts.idle_time_limit)
            .max_user_time(opts.time_limit)
            .max_memory_usage(mb2b(opts.memory_limit))
            .max_output_size(mb2b(opts.write_limit))
            .max_processes(opts.process_count as u64)
            .monitor_interval(opts.monitor_interval)
            .show_gui(opts.show_window)
            .build()
    }
}

fn json_report(reports: &Result<Vec<Report>>, idx: usize, opts: &Options) -> JsonValue {
    let mut obj = object! {
        "Application" => opts.argv[0].clone(),
        "Arguments" => opts.argv[1..].to_vec(),
        "Limit" => json_limits(opts),
        "Options" => object! {
            "SearchInPath" => opts.use_syspath
        },
        "StdIn" => json_redirect_list(&opts.stdin_redirect),
        "StdOut" => json_redirect_list(&opts.stdout_redirect),
        "StdErr" => json_redirect_list(&opts.stderr_redirect),
        "CreateProcessMethod" => "CreateProcess",
    };

    match reports {
        Ok(reports) => {
            let report = &reports[idx];
            obj["Result"] = object! {
                "Time" => dur_to_fsec(&report.statistics.total_user_time),
                "WallClockTime" => dur_to_fsec(&report.statistics.wall_clock_time),
                "Memory" => report.statistics.peak_memory_used,
                "BytesWritten" => report.statistics.total_bytes_written,
                "KernelTime" =>  dur_to_fsec(&report.statistics.total_kernel_time),
                // C++ spawner computes processor load as total user time / wall clock time.
                // Making it possible for the processor load to be greater than 1.0
                "ProcessorLoad" =>
                    dur_to_fsec(&report.statistics.total_user_time) / dur_to_fsec(&report.statistics.wall_clock_time),
                "WorkingDirectory" => report.command.current_dir.clone(),
            };
            obj["UserName"] = "todo".into();
            match report.exit_status {
                ExitStatus::Finished(code) => {
                    obj["TerminateReason"] = "ExitProcess".into();
                    obj["ExitCode"] = code.into();
                    obj["ExitStatus"] = code.to_string().into();
                }
                ExitStatus::Terminated(ref r) => {
                    obj["TerminateReason"] = json_term_reason(r);
                    obj["ExitCode"] = 0.into();
                    obj["ExitStatus"] = "0".into();
                }
            }
            obj["SpawnerError"] = array!["<none>"];
        }
        Err(e) => {
            obj["Result"] = object! {
                "Time" => 0.0,
                "WallClockTime" => 0.0,
                "Memory" => 0,
                "BytesWritten" => 0,
                "KernelTime" => 0.0,
                "ProcessorLoad" => 0.0,
                "WorkingDirectory" => "",
            };
            obj["UserName"] = "".into();
            obj["TerminateReason"] = "ExitProcess".into();
            obj["ExitCode"] = 0.into();
            obj["ExitStatus"] = "0".into();
            obj["SpawnerError"] = array![e.to_string()];
        }
    }
    array![obj]
}

fn dur_to_fsec(d: &Duration) -> f64 {
    let us = d.as_secs() as f64 * 1e6 + d.subsec_micros() as f64;
    us / 1e6
}

fn json_limits(opts: &Options) -> JsonValue {
    let default_opts = Options::default();
    let mut limits = JsonValue::new_object();
    if opts.time_limit != default_opts.time_limit {
        limits["Time"] = dur_to_fsec(&opts.time_limit).into();
    }
    if opts.wall_clock_time_limit != default_opts.wall_clock_time_limit {
        limits["WallClockTime"] = dur_to_fsec(&opts.wall_clock_time_limit).into();
    }
    if opts.memory_limit != default_opts.memory_limit {
        limits["Memory"] = opts.memory_limit.into();
    }
    if opts.secure != default_opts.secure {
        limits["SecurityLevel"] = (opts.secure as u32).into();
    }
    if opts.write_limit != default_opts.write_limit {
        limits["IOBytes"] = opts.write_limit.into();
    }
    if opts.idle_time_limit != default_opts.idle_time_limit {
        limits["IdlenessTime"] = dur_to_fsec(&opts.idle_time_limit).into();
    }

    // The "IdlenessProcessorLoad" seems to be always present in c++ spawner.
    limits["IdlenessProcessorLoad"] = opts.load_ratio.into();
    limits
}

fn json_redirect_list(list: &StdioRedirectList) -> JsonValue {
    list.items
        .iter()
        .map(|x| match &x.kind {
            StdioRedirectKind::Pipe(p) => format!("*{}", p.to_string()),
            StdioRedirectKind::File(f) => f.clone(),
        })
        .collect::<Vec<String>>()
        .into()
}

fn json_term_reason(r: &TerminationReason) -> JsonValue {
    match r {
        TerminationReason::WallClockTimeLimitExceeded => "TimeLimitExceeded",
        TerminationReason::IdleTimeLimitExceeded => "IdleTimeLimitExceeded",
        TerminationReason::UserTimeLimitExceeded => "TimeLimitExceeded",
        TerminationReason::WriteLimitExceeded => "WriteLimitExceeded",
        TerminationReason::MemoryLimitExceeded => "MemoryLimitExceeded",
        TerminationReason::ProcessLimitExceeded => "ProcessesCountLimitExceeded",
        TerminationReason::Other => "TerminatedByController",
    }
    .into()
}
