use crate::Result;
use command::Command;
use pipe::{ReadPipe, WritePipe};
use process::Stdio;
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

#[derive(Copy, Clone)]
pub struct IstreamIndex(usize);

#[derive(Copy, Clone)]
pub struct OstreamIndex(usize);

enum IstreamSrcKind {
    Pipe(ReadPipe),
    File(PathBuf),
    Ostream(OstreamIndex),
}

pub struct IstreamSrc {
    kind: IstreamSrcKind,
}

pub enum OstreamDstKind {
    Pipe(WritePipe),
    File(PathBuf),
    Istream(IstreamIndex),
}

pub struct OstreamDst {
    kind: OstreamDstKind,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: IstreamIndex,
    pub stdout: OstreamIndex,
    pub stderr: OstreamIndex,
}

pub struct SessionBuilder {
    cmds: Vec<Command>,
    stdio_mappings: Vec<StdioMapping>,
    builder: RouterBuilder,
    output_files: HashMap<PathBuf, usize>,
}

impl Session {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn wait(mut self) -> Result<Vec<RunnerReport>> {
        self.wait_impl()
    }

    fn wait_impl(&mut self) -> Result<Vec<RunnerReport>> {
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

impl Drop for Session {
    fn drop(&mut self) {
        for runner in self.runners.drain(..) {
            runner.kill();
        }
        let _ = self.wait_impl();
    }
}

impl IstreamSrc {
    pub fn pipe(p: ReadPipe) -> Self {
        Self {
            kind: IstreamSrcKind::Pipe(p),
        }
    }

    pub fn ostream(ostream: OstreamIndex) -> Self {
        Self {
            kind: IstreamSrcKind::Ostream(ostream),
        }
    }

    pub fn file<P: AsRef<Path>>(path: P) -> Self {
        Self {
            kind: IstreamSrcKind::File(path.as_ref().to_path_buf()),
        }
    }
}

impl OstreamDst {
    pub fn pipe(p: WritePipe) -> Self {
        Self {
            kind: OstreamDstKind::Pipe(p),
        }
    }

    pub fn istream(istream: IstreamIndex) -> Self {
        Self {
            kind: OstreamDstKind::Istream(istream),
        }
    }

    pub fn file<P: AsRef<Path>>(path: P) -> Self {
        Self {
            kind: OstreamDstKind::File(path.as_ref().to_path_buf()),
        }
    }
}

impl SessionBuilder {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            stdio_mappings: Vec::new(),
            builder: RouterBuilder::new(),
            output_files: HashMap::new(),
        }
    }

    pub fn add_cmd(&mut self, cmd: Command) -> StdioMapping {
        let stdio = StdioMapping {
            stdin: IstreamIndex(self.builder.add_istream(None)),
            stdout: OstreamIndex(self.builder.add_ostream(None)),
            stderr: OstreamIndex(self.builder.add_ostream(None)),
        };
        self.stdio_mappings.push(stdio);
        self.cmds.push(cmd);
        stdio
    }

    pub fn add_istream_src(&mut self, istream: IstreamIndex, src: IstreamSrc) -> Result<()> {
        let ostream = match src.kind {
            IstreamSrcKind::Pipe(p) => self.builder.add_ostream(Some(p)),
            IstreamSrcKind::File(f) => self.builder.add_ostream(Some(ReadPipe::open(f)?)),
            IstreamSrcKind::Ostream(i) => i.0,
        };
        self.builder.connect(istream.0, ostream)
    }

    pub fn add_ostream_dst(&mut self, ostream: OstreamIndex, dst: OstreamDst) -> Result<()> {
        let istream = match dst.kind {
            OstreamDstKind::Pipe(p) => self.builder.add_istream(Some(p)),
            OstreamDstKind::File(f) => match self.output_files.entry(f) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    let pipe = WritePipe::open(e.key())?;
                    *e.insert(self.builder.add_istream(Some(pipe)))
                }
            },
            OstreamDstKind::Istream(i) => i.0,
        };
        self.builder.connect(istream, ostream.0)
    }

    pub fn spawn(mut self) -> Result<Session> {
        let (router, mut iolist) = self.builder.spawn()?;
        let mut sess = Session {
            router: router,
            runners: Vec::new(),
            runner_threads: Vec::new(),
        };

        for (cmd, mapping) in self.cmds.drain(..).zip(self.stdio_mappings.drain(..)) {
            let thread = runner_private::spawn(
                cmd,
                Stdio {
                    stdin: iolist.istreams[mapping.stdin.0].take(),
                    stdout: iolist.ostreams[mapping.stdout.0].take(),
                    stderr: iolist.ostreams[mapping.stderr.0].take(),
                },
            )?;
            sess.runners.push(thread.runner().clone());
            sess.runner_threads.push(thread);
        }

        Ok(sess)
    }
}
