pub mod opts;
mod report;
mod value_parser;

#[cfg(test)]
mod tests;

pub use self::report::*;

use self::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use crate::{Error, Result};
use command::{Command, CommandBuilder, CommandCallbacks, Limits};
use driver::prelude::*;
use json::JsonValue;
use session::{IstreamDst, IstreamIdx, OstreamIdx, OstreamSrc, SessionBuilder, StdioMapping};
use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::u64;

pub struct Driver {
    pub controller: Option<usize>,
    pub cmds: Vec<Options>,
    builder: RefCell<SessionBuilder>,
    stdio_mappings: Vec<StdioMapping>,
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

    Driver::create(controller, cmds)
}

pub fn run<T, U>(argv: T) -> Result<Report>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let driver = parse(argv)?;
    Ok(driver.run())
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

    let report = driver.run();
    let mut output_files: HashMap<&String, Vec<CommandReportKind>> = HashMap::new();
    for (idx, cmd) in report.cmds.iter().enumerate() {
        let cmd_report = report.at(idx);
        if !cmd.hide_report && report.cmds.len() == 1 {
            println!("{}", cmd_report);
        }
        if let Some(filename) = &cmd.output_file {
            output_files
                .entry(filename)
                .or_insert(Vec::new())
                .push(cmd_report.kind());
        }
    }

    for (filename, report_kinds) in output_files.into_iter() {
        let _ = fs::remove_file(filename);
        let mut file = match fs::File::create(filename) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{}", e);
                continue;
            }
        };

        if report_kinds.len() == 1 && !report_kinds[0].is_json() {
            let _ = write!(&mut file, "{}", report_kinds[0]);
        } else if report_kinds.iter().all(|k| k.is_json()) {
            let reports =
                JsonValue::Array(report_kinds.into_iter().map(|k| k.into_json()).collect());
            let _ = reports.write_pretty(&mut file, 4);
        }
    }
}

impl Driver {
    fn create(controller: Option<usize>, cmds: Vec<Options>) -> Result<Driver> {
        let mut driver = Driver {
            controller: controller,
            cmds: cmds,
            builder: RefCell::new(SessionBuilder::new()),
            stdio_mappings: Vec::new(),
        };

        for cmd in driver.cmds.iter() {
            driver.stdio_mappings.push(
                driver
                    .builder
                    .borrow_mut()
                    .add_cmd(Command::from(cmd), CommandCallbacks::none()),
            );
        }

        for (cmd, mapping) in driver.cmds.iter().zip(driver.stdio_mappings.iter()) {
            driver.redirect_ostream(mapping.stdin, &cmd.stdin_redirect)?;
            driver.redirect_istream(mapping.stdout, &cmd.stdout_redirect)?;
            driver.redirect_istream(mapping.stderr, &cmd.stderr_redirect)?;
        }

        Ok(driver)
    }

    fn redirect_istream(
        &self,
        istream: IstreamIdx,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        let stdio_len = self.stdio_mappings.len();
        for redirect in redirect_list.items.iter() {
            match &redirect.kind {
                StdioRedirectKind::File(s) => {
                    self.builder
                        .borrow_mut()
                        .add_istream_dst(istream, IstreamDst::file(s))?;
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Null => { /* pipes are null by default */ }
                    PipeKind::Std => { /* todo */ }
                    PipeKind::Stdin(i) => {
                        if *i >= stdio_len {
                            return Err(Error::from(format!("stdin index {} is out of range", i)));
                        }
                        self.builder.borrow_mut().add_istream_dst(
                            istream,
                            IstreamDst::ostream(self.stdio_mappings[*i].stdin),
                        )?;
                    }
                    PipeKind::Stderr(i) => {
                        if *i >= stdio_len {
                            return Err(Error::from(format!("stderr index {} is out of range", i)));
                        }
                        self.builder.borrow_mut().add_istream_dst(
                            istream,
                            IstreamDst::ostream(self.stdio_mappings[*i].stdin),
                        )?;
                    }
                    _ => {}
                },
            }
        }
        Ok(())
    }

    fn redirect_ostream(
        &self,
        ostream: OstreamIdx,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            match &redirect.kind {
                StdioRedirectKind::File(s) => {
                    self.builder
                        .borrow_mut()
                        .add_ostream_src(ostream, OstreamSrc::file(s))?;
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Null => { /* pipes are null by default */ }
                    PipeKind::Std => { /* todo */ }
                    PipeKind::Stdout(i) => {
                        if *i >= self.stdio_mappings.len() {
                            return Err(Error::from(format!("stdout index {} is out of range", i)));
                        }
                        self.builder.borrow_mut().add_ostream_src(
                            ostream,
                            OstreamSrc::istream(self.stdio_mappings[*i].stdout),
                        )?;
                    }
                    _ => {}
                },
            }
        }
        Ok(())
    }

    pub fn run(self) -> Report {
        Report {
            runner_reports: self
                .builder
                .into_inner()
                .spawn()
                .and_then(|sess| sess.wait()),
            cmds: self.cmds,
        }
    }
}

pub(crate) fn mb2b(mb: f64) -> u64 {
    let b = mb * 1024.0 * 1024.0;
    if b.is_infinite() {
        u64::MAX
    } else {
        b as u64
    }
}

impl From<&Options> for Command {
    fn from(opts: &Options) -> Command {
        CommandBuilder::new(opts.argv[0].clone())
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
