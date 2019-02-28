pub mod opts;
mod protocol;
mod report;
mod value_parser;

#[cfg(test)]
mod tests;

pub use self::report::*;

use self::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use self::protocol::{
    AgentIdx, AgentStdout, AgentTermination, CommandIdx, Context, ControllerStdin,
    ControllerStdout, ControllerTermination,
};
use crate::{Error, Result};
use command::{CommandBuilder, CommandController, Limits};
use driver::prelude::*;
use json::JsonValue;
use pipe::{self, ReadPipe};
use session::{IstreamDst, OstreamSrc, Session, SessionBuilder, StdioMapping};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::u64;
use stdio::{IstreamIdx, OstreamIdx};

pub enum CommandKind {
    Default,
    Controller,
    Agent(AgentIdx),
}

pub struct CommandInfo {
    pub opts: Options,
    pub kind: CommandKind,
}

pub struct Driver {
    pub cmds: Vec<CommandInfo>,
}

struct SessionBuilderEx {
    base: SessionBuilder,
    ctx: Context,
    mappings: Vec<StdioMapping>,
    controller_stdin_r: ReadPipe,
    controller_stdin_w: ControllerStdin,
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

    Ok(Driver::new(controller, cmds))
}

pub fn run<T, U>(argv: T) -> Result<Report>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let report = parse(argv).and_then(|driver| driver.run())?;
    if report.cmds.len() == 0 {
        println!("{}", Options::help());
        return Ok(report);
    }

    print_report(&report)?;
    Ok(report)
}

pub fn main() {
    if let Err(e) = run(std::env::args().skip(1)) {
        eprintln!("{}", e);
    }
}

fn print_report(report: &Report) -> io::Result<()> {
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

    for (filename, kinds) in output_files.into_iter() {
        let _ = fs::remove_file(filename);
        let mut file = fs::File::create(filename)?;
        if kinds.len() == 1 && !kinds[0].is_json() {
            write!(&mut file, "{}", kinds[0])?;
        } else if kinds.iter().all(|k| k.is_json()) {
            let reports = JsonValue::Array(kinds.into_iter().map(|k| k.into_json()).collect());
            reports.write_pretty(&mut file, 4)?;
        }
    }
    Ok(())
}

impl CommandKind {
    pub fn is_agent(&self) -> bool {
        match self {
            CommandKind::Agent(_) => true,
            _ => false,
        }
    }
    pub fn is_controller(&self) -> bool {
        match self {
            CommandKind::Controller => true,
            _ => false,
        }
    }
}

impl Driver {
    fn new(controller_idx: Option<usize>, opts: Vec<Options>) -> Driver {
        let mut driver = Driver { cmds: Vec::new() };
        let mut agent_idx = 0;

        for (idx, opts) in opts.into_iter().enumerate() {
            let is_agent = !opts.controller;
            driver.cmds.push(CommandInfo {
                opts: opts,
                kind: match controller_idx {
                    Some(controller_idx) => {
                        if idx == controller_idx {
                            CommandKind::Controller
                        } else {
                            CommandKind::Agent(AgentIdx(agent_idx))
                        }
                    }
                    None => CommandKind::Default,
                },
            });

            if is_agent {
                agent_idx += 1;
            }
        }

        driver
    }

    pub fn run(self) -> Result<Report> {
        let mut builder = SessionBuilderEx::create()?;
        builder.add_cmds(&self);
        builder.setup_stdio(&self)?;

        Ok(Report {
            runner_reports: builder.spawn(&self)?.wait(),
            cmds: self.cmds.into_iter().map(|cmd| cmd.opts).collect(),
        })
    }
}

impl SessionBuilderEx {
    fn create() -> Result<Self> {
        pipe::create().map(|(r, w)| Self {
            base: SessionBuilder::new(),
            mappings: Vec::new(),
            ctx: Context::new(),
            controller_stdin_r: r,
            controller_stdin_w: ControllerStdin::new(w),
        })
    }

    fn add_cmds(&mut self, driver: &Driver) {
        for (cmd_idx, cmd_info) in driver.cmds.iter().enumerate() {
            let opts = &cmd_info.opts;
            let cmd = CommandBuilder::new(opts.argv[0].clone())
                .args(opts.argv.iter().skip(1))
                .env_kind(opts.env)
                .env_vars(&opts.env_vars)
                .monitor_interval(opts.monitor_interval)
                .show_gui(opts.show_window)
                .spawn_suspended(cmd_info.kind.is_agent())
                .limits(Limits {
                    max_wall_clock_time: opts.wall_clock_time_limit,
                    max_idle_time: opts.idle_time_limit,
                    max_user_time: opts.time_limit,
                    max_memory_usage: opts.memory_limit.map(|v| mb2b(v)),
                    max_output_size: opts.write_limit.map(|v| mb2b(v)),
                    max_processes: opts.process_count,
                })
                .current_dir_opt(opts.working_directory.as_ref())
                .build();

            let ctl = self.cmd_controller(cmd_idx, &driver);
            let mapping = self.base.add_cmd(cmd, ctl);
            self.mappings.push(mapping);
        }
    }

    fn setup_stdio(&mut self, driver: &Driver) -> Result<()> {
        for (idx, cmd) in driver.cmds.iter().enumerate() {
            let mapping = self.mappings[idx];
            self.redirect_ostream(mapping.stdin, &cmd.opts.stdin_redirect)?;
            self.redirect_istream(mapping.stdout, &cmd.opts.stdout_redirect)?;
            self.redirect_istream(mapping.stderr, &cmd.opts.stderr_redirect)?;
        }
        Ok(())
    }

    fn spawn(mut self, driver: &Driver) -> Result<Session> {
        if let Some(controller_idx) = driver.cmds.iter().position(|cmd| cmd.kind.is_controller()) {
            let stdin = self.mappings[controller_idx].stdin;
            self.base
                .add_ostream_src(stdin, OstreamSrc::pipe(self.controller_stdin_r))?;
        }

        let sess = self.base.spawn()?;
        self.ctx.init(sess.runners(), self.mappings);
        Ok(sess)
    }

    fn cmd_controller(&self, cmd_idx: usize, driver: &Driver) -> CommandController {
        let cmds = &driver.cmds;
        match cmds[cmd_idx].kind {
            CommandKind::Default => CommandController {
                on_terminate: None,
                stdout_controller: None,
            },
            CommandKind::Agent(agent_idx) => CommandController {
                on_terminate: Some(Box::new(AgentTermination::new(
                    agent_idx,
                    self.controller_stdin_w.clone(),
                ))),
                stdout_controller: Some(Box::new(AgentStdout::new(
                    self.ctx.clone(),
                    agent_idx,
                    CommandIdx(cmd_idx),
                ))),
            },
            CommandKind::Controller => {
                let agent_indices: Vec<CommandIdx> = (0..cmds.len())
                    .filter_map(|i| {
                        if cmds[i].kind.is_agent() {
                            Some(CommandIdx(i))
                        } else {
                            None
                        }
                    })
                    .collect();
                CommandController {
                    on_terminate: Some(Box::new(ControllerTermination::new(
                        self.ctx.clone(),
                        agent_indices.clone(),
                    ))),
                    stdout_controller: Some(Box::new(ControllerStdout::new(
                        self.ctx.clone(),
                        CommandIdx(cmd_idx),
                        agent_indices,
                    ))),
                }
            }
        }
    }

    fn redirect_istream(
        &mut self,
        istream: IstreamIdx,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let dst = match &redirect.kind {
                StdioRedirectKind::File(f) => {
                    Some(IstreamDst::file(f, redirect.flags.share_mode()))
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Stdin(i) => {
                        self.check_stdio_idx("stdin", *i)?;
                        Some(IstreamDst::ostream(self.mappings[*i].stdin))
                    }
                    PipeKind::Stderr(i) => {
                        self.check_stdio_idx("stderr", *i)?;
                        Some(IstreamDst::ostream(self.mappings[*i].stdin))
                    }
                    _ => None,
                },
            };
            if let Some(dst) = dst {
                self.base.add_istream_dst(istream, dst)?;
            }
        }
        Ok(())
    }

    fn redirect_ostream(
        &mut self,
        ostream: OstreamIdx,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let src = match &redirect.kind {
                StdioRedirectKind::File(f) => {
                    Some(OstreamSrc::file(f, redirect.flags.share_mode()))
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Stdout(i) => {
                        self.check_stdio_idx("stdout", *i)?;
                        Some(OstreamSrc::istream(self.mappings[*i].stdout))
                    }
                    _ => None,
                },
            };
            if let Some(src) = src {
                self.base.add_ostream_src(ostream, src)?;
            }
        }
        Ok(())
    }

    fn check_stdio_idx(&self, stream: &str, idx: usize) -> Result<()> {
        if idx >= self.mappings.len() {
            Err(Error::from(format!("{} index is out of range", stream)))
        } else {
            Ok(())
        }
    }
}

fn mb2b(mb: f64) -> u64 {
    let b = mb * 1024.0 * 1024.0;
    if b.is_infinite() {
        u64::MAX
    } else {
        b as u64
    }
}
