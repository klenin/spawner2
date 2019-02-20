pub use stdio::router::{IstreamIdx, OstreamIdx};

use crate::Result;
use command::{Command, CommandCallbacks};
use pipe::{ReadPipe, WritePipe};
use process::ProcessStdio;
use runner::{Runner, RunnerReport};
use runner_private::{self, RunnerThread};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use stdio::router::{Router, RouterBuilder};

pub struct Session {
    router: Router,
    runner_threads: Vec<RunnerThread>,
    runners: Vec<Runner>,
}

enum IstreamDstKind {
    Pipe(WritePipe),
    File(PathBuf),
    Ostream(OstreamIdx),
}

pub struct IstreamDst {
    kind: IstreamDstKind,
}

enum OstreamSrcKind {
    Pipe(ReadPipe),
    File(PathBuf),
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

struct Target {
    cmd: Command,
    cbs: CommandCallbacks,
    stdio_mapping: StdioMapping,
}

pub struct SessionBuilder {
    targets: Vec<Target>,
    builder: RouterBuilder,
    output_files: HashMap<PathBuf, OstreamIdx>,
}

impl Session {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn wait(mut self) -> Result<Vec<RunnerReport>> {
        let mut reports: Vec<RunnerReport> = Vec::new();
        for thread in self.runner_threads.drain(..) {
            reports.push(thread.join()?);
        }

        // It is (almost) impossible to hang on this because all pipes
        // (except user-provided ones) are dead at this point.
        self.router.stop()?;
        Ok(reports)
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

    pub fn file<P: AsRef<Path>>(path: P) -> Self {
        Self {
            kind: IstreamDstKind::File(path.as_ref().to_path_buf()),
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

    pub fn file<P: AsRef<Path>>(path: P) -> Self {
        Self {
            kind: OstreamSrcKind::File(path.as_ref().to_path_buf()),
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

    pub fn add_cmd(&mut self, cmd: Command, cbs: CommandCallbacks) -> StdioMapping {
        let mapping = StdioMapping {
            stdin: self.builder.add_ostream(None),
            stdout: self.builder.add_istream(None),
            stderr: self.builder.add_istream(None),
        };
        self.targets.push(Target {
            cmd: cmd,
            cbs: cbs,
            stdio_mapping: mapping,
        });
        mapping
    }

    pub fn add_istream_dst(&mut self, istream: IstreamIdx, dst: IstreamDst) -> Result<()> {
        let ostream = match dst.kind {
            IstreamDstKind::Pipe(p) => self.builder.add_ostream(Some(p)),
            IstreamDstKind::File(f) => match self.output_files.entry(f) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    let pipe = WritePipe::open(e.key())?;
                    *e.insert(self.builder.add_ostream(Some(pipe)))
                }
            },
            IstreamDstKind::Ostream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn add_ostream_src(&mut self, ostream: OstreamIdx, src: OstreamSrc) -> Result<()> {
        let istream = match src.kind {
            OstreamSrcKind::Pipe(p) => self.builder.add_istream(Some(p)),
            OstreamSrcKind::File(f) => self.builder.add_istream(Some(ReadPipe::open(f)?)),
            OstreamSrcKind::Istream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn spawn(self) -> Result<Session> {
        let (mut iolist, router) = self.builder.spawn()?;
        let mut sess = Session {
            router: router,
            runners: Vec::new(),
            runner_threads: Vec::new(),
        };

        for target in self.targets.into_iter() {
            let mapping = target.stdio_mapping;
            let thread = runner_private::spawn(
                target.cmd,
                target.cbs,
                ProcessStdio {
                    stdin: iolist.ostream_dsts[mapping.stdin.0].take().unwrap(),
                    stdout: iolist.istream_srcs[mapping.stdout.0].take().unwrap(),
                    stderr: iolist.istream_srcs[mapping.stderr.0].take().unwrap(),
                },
            )?;
            sess.runners.push(thread.runner().clone());
            sess.runner_threads.push(thread);
        }

        Ok(sess)
    }
}
