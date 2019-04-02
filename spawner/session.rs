use crate::command::{Command, CommandController, OnTerminate};
use crate::pipe::{ReadPipe, ShareMode, WritePipe};
use crate::runner::{Runner, RunnerReport};
use crate::runner_private::{MonitoringLoop, ProcessStdio, RunnerThread};
use crate::stdio::router::{Router, RouterBuilder};
use crate::stdio::{IstreamIdx, OstreamIdx};
use crate::{Error, Result};

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub struct Session {
    router: Router,
    runner_threads: Vec<RunnerThread>,
    stdio_mappings: Vec<StdioMapping>,
}

enum IstreamDstKind {
    Pipe(WritePipe),
    File(PathBuf, ShareMode),
    Ostream(OstreamIdx),
}

pub struct IstreamDst {
    kind: IstreamDstKind,
}

enum OstreamSrcKind {
    Pipe(ReadPipe),
    File(PathBuf, ShareMode),
    Istream(IstreamIdx),
}

pub struct OstreamSrc {
    kind: OstreamSrcKind,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: OstreamIdx,
    pub stdout: IstreamIdx,
    pub stderr: IstreamIdx,
}

pub struct SessionBuilder {
    targets: Vec<Target>,
    builder: RouterBuilder,
    output_files: HashMap<PathBuf, OstreamIdx>,
}

#[derive(Debug)]
pub struct CommandErrors {
    pub errors: Vec<Error>,
}

pub type CommandResult = std::result::Result<RunnerReport, CommandErrors>;

struct Target {
    cmd: Command,
    on_terminate: Option<Box<OnTerminate>>,
    stdio_mapping: StdioMapping,
}

struct ThreadData {
    ml: MonitoringLoop,
    on_terminate: Option<Box<OnTerminate>>,
    mapping: StdioMapping,
}

impl Session {
    pub fn runners(&self) -> Vec<Runner> {
        self.runner_threads
            .iter()
            .map(|t| t.runner().clone())
            .collect()
    }

    pub fn wait(self) -> Vec<CommandResult> {
        let mut results: Vec<CommandResult> = self
            .runner_threads
            .into_iter()
            .map(|thread| thread.join().map_err(|e| CommandErrors { errors: vec![e] }))
            .collect();

        let mut stop_errors = self.router.stop();
        for (idx, mapping) in self.stdio_mappings.into_iter().enumerate() {
            for istream in [&mapping.stdout, &mapping.stderr].iter() {
                let err = match stop_errors.istream_errors.remove(istream) {
                    Some(e) => e,
                    None => continue,
                };
                if results[idx].is_ok() {
                    results[idx] = Err(CommandErrors { errors: vec![err] });
                } else {
                    results[idx].as_mut().unwrap_err().errors.push(err);
                }
            }
        }
        results
    }
}

impl IstreamDst {
    pub fn pipe(p: WritePipe) -> Self {
        Self {
            kind: IstreamDstKind::Pipe(p),
        }
    }

    pub fn ostream(ostream: OstreamIdx) -> Self {
        Self {
            kind: IstreamDstKind::Ostream(ostream),
        }
    }

    pub fn file<P: AsRef<Path>>(path: P, mode: ShareMode) -> Self {
        Self {
            kind: IstreamDstKind::File(path.as_ref().to_path_buf(), mode),
        }
    }
}

impl OstreamSrc {
    pub fn pipe(p: ReadPipe) -> Self {
        Self {
            kind: OstreamSrcKind::Pipe(p),
        }
    }

    pub fn istream(istream: IstreamIdx) -> Self {
        Self {
            kind: OstreamSrcKind::Istream(istream),
        }
    }

    pub fn file<P: AsRef<Path>>(path: P, mode: ShareMode) -> Self {
        Self {
            kind: OstreamSrcKind::File(path.as_ref().to_path_buf(), mode),
        }
    }
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self {
            targets: Vec::new(),
            builder: RouterBuilder::new(),
            output_files: HashMap::new(),
        }
    }

    pub fn add_cmd(&mut self, cmd: Command, ctl: CommandController) -> StdioMapping {
        let mapping = StdioMapping {
            stdin: self.builder.add_ostream(None),
            stdout: self.builder.add_istream(None, ctl.stdout_controller),
            stderr: self.builder.add_istream(None, None),
        };
        self.targets.push(Target {
            cmd: cmd,
            on_terminate: ctl.on_terminate,
            stdio_mapping: mapping,
        });
        mapping
    }

    pub fn add_istream_dst(&mut self, istream: IstreamIdx, dst: IstreamDst) -> Result<()> {
        let ostream = match dst.kind {
            IstreamDstKind::Pipe(p) => self.builder.add_ostream(Some(p)),
            IstreamDstKind::File(file, mode) => match self.output_files.entry(file) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    let pipe = WritePipe::open(e.key(), mode)?;
                    *e.insert(self.builder.add_ostream(Some(pipe)))
                }
            },
            IstreamDstKind::Ostream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn add_ostream_src(&mut self, ostream: OstreamIdx, src: OstreamSrc) -> Result<()> {
        let istream = match src.kind {
            OstreamSrcKind::Pipe(p) => self.builder.add_istream(Some(p), None),
            OstreamSrcKind::File(file, mode) => self
                .builder
                .add_istream(Some(ReadPipe::open(file, mode)?), None),
            OstreamSrcKind::Istream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn spawn(self) -> Result<Session> {
        let (mut iolist, router) = self.builder.spawn()?;
        let mut sess = Session {
            router: router,
            runner_threads: Vec::new(),
            stdio_mappings: Vec::new(),
        };

        let mapped_targets = self.targets.into_iter().map(|target| {
            let mapping = target.stdio_mapping;
            let on_terminate = target.on_terminate;
            MonitoringLoop::create(
                target.cmd,
                ProcessStdio {
                    stdin: iolist.ostream_dsts[mapping.stdin.0].take().unwrap(),
                    stdout: iolist.istream_srcs[mapping.stdout.0].take().unwrap(),
                    stderr: iolist.istream_srcs[mapping.stderr.0].take().unwrap(),
                },
            )
            .map(|ml| ThreadData {
                ml: ml,
                mapping: mapping,
                on_terminate: on_terminate,
            })
        });

        for thread_data_result in mapped_targets {
            let thread_data = thread_data_result?;
            sess.stdio_mappings.push(thread_data.mapping);
            sess.runner_threads.push(RunnerThread::spawn(
                thread_data.ml,
                thread_data.on_terminate,
            )?);
        }

        Ok(sess)
    }
}

impl std::error::Error for CommandErrors {}

impl fmt::Display for CommandErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.errors.iter() {
            write!(f, "{}\n", e)?;
        }
        Ok(())
    }
}
