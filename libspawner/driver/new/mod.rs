pub mod opts;
mod report;
mod value_parser;

#[cfg(test)]
mod tests;

use self::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use self::report::ReportKind;
use crate::{Error, Result};
use command::{self, Command, Limits};
use driver::prelude::*;
use json::{stringify_pretty, JsonValue};
use runner::Report;
use session::{IstreamSrc, OstreamDst, Session, StdioMapping};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
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
    let mut output_files: HashMap<&String, Vec<ReportKind>> = HashMap::new();
    for (idx, cmd) in driver.cmds.iter().enumerate() {
        let report = report::create(&reports, idx, cmd);
        if !cmd.hide_report && driver.cmds.len() == 1 {
            println!("{}", report.to_string());
        }
        if let Some(filename) = &cmd.output_file {
            output_files
                .entry(filename)
                .or_insert(Vec::new())
                .push(report);
        }
    }

    for (filename, reports) in output_files.into_iter() {
        let _ = fs::remove_file(filename);
        let mut file = match fs::File::create(filename) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{}", e);
                continue;
            }
        };

        if reports.len() == 1 {
            let _ = write!(&mut file, "{}", reports[0].to_string());
            continue;
        }

        if reports.iter().all(|r| r.is_json()) {
            let array = JsonValue::Array(reports.into_iter().map(|r| r.into_json()).collect());
            let _ = write!(&mut file, "{}", stringify_pretty(array, 4));
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

pub(crate) fn mb2b(mb: f64) -> u64 {
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
            .env_kind(opts.env)
            .env_vars(&opts.env_vars)
            .monitor_interval(opts.monitor_interval)
            .show_gui(opts.show_window)
            .limits(Limits {
                max_wall_clock_time: opts.wall_clock_time_limit,
                max_idle_time: opts.idle_time_limit,
                max_user_time: opts.time_limit,
                max_memory_usage: opts.memory_limit.map(|v| mb2b(v)),
                max_output_size: opts.write_limit.map(|v| mb2b(v)),
                max_processes: opts.process_count,
            })
            .current_dir_opt(opts.working_directory.as_ref())
            .build()
    }
}
