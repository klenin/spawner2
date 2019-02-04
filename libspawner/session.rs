use crate::Result;
use command::Command;
use pipe::{ReadPipe, WritePipe};
use process::Stdio;
use runner::{Report, Runner};
use runner_private::{run, WaitHandle};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use stdio::router::{self, Builder};

pub struct Session {
    cmds: Vec<Command>,
    stdio_mappings: Vec<StdioMapping>,
    builder: Builder,
    output_files: HashMap<String, usize>,
}

pub enum IstreamSrc<'a> {
    Pipe(ReadPipe),
    File(&'a str),
    Ostream(usize),
}

pub enum OstreamDst<'a> {
    Pipe(WritePipe),
    File(&'a str),
    Istream(usize),
}

pub struct Spawner {
    router: router::StopHandle,
    runner_handles: Vec<WaitHandle>,
    runners: Vec<Runner>,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: usize,
    pub stdout: usize,
    pub stderr: usize,
}

impl Session {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            stdio_mappings: Vec::new(),
            builder: Builder::new(),
            output_files: HashMap::new(),
        }
    }

    pub fn add_cmd(&mut self, cmd: Command) -> StdioMapping {
        let stdio = StdioMapping {
            stdin: self.builder.add_unknown_istream(),
            stdout: self.builder.add_unknown_ostream(),
            stderr: self.builder.add_unknown_ostream(),
        };
        self.stdio_mappings.push(stdio);
        self.cmds.push(cmd);
        stdio
    }

    pub fn connect_istream(&mut self, istream: usize, src: IstreamSrc) -> Result<()> {
        let ostream = match src {
            IstreamSrc::Pipe(p) => self.builder.add_file_ostream(p),
            IstreamSrc::File(f) => self.builder.add_file_ostream(ReadPipe::open(f)?),
            IstreamSrc::Ostream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn connect_ostream(&mut self, ostream: usize, dst: OstreamDst) -> Result<()> {
        let istream = match dst {
            OstreamDst::Pipe(p) => self.builder.add_file_istream(p),
            OstreamDst::File(f) => match self.output_files.entry(f.to_string()) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => *e.insert(self.builder.add_file_istream(WritePipe::open(f)?)),
            },
            OstreamDst::Istream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn spawn(mut self) -> Result<Spawner> {
        let (router, mut list) = self.builder.build()?;
        let mut sp = Spawner {
            router: router.start()?,
            runners: Vec::new(),
            runner_handles: Vec::new(),
        };

        for (cmd, mapping) in self.cmds.drain(..).zip(self.stdio_mappings.drain(..)) {
            let handle = run(
                cmd,
                Stdio {
                    stdin: list.istreams[mapping.stdin].take(),
                    stdout: list.ostreams[mapping.stdout].take(),
                    stderr: list.ostreams[mapping.stderr].take(),
                },
            )?;
            sp.runners.push(handle.runner().clone());
            sp.runner_handles.push(handle);
        }

        Ok(sp)
    }
}

impl Spawner {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn wait(mut self) -> Result<Vec<Report>> {
        self.wait_impl()
    }

    fn wait_impl(&mut self) -> Result<Vec<Report>> {
        let mut reports: Vec<Report> = Vec::new();
        for runner in self.runner_handles.drain(..) {
            reports.push(runner.wait()?);
        }

        // It is (almost) impossible to hang on this because all pipes
        // (except user-provided ones) are dead at this point.
        self.router.stop()?;
        Ok(reports)
    }
}

impl Drop for Spawner {
    fn drop(&mut self) {
        for runner in self.runners.drain(..) {
            runner.kill();
        }
        let _ = self.wait_impl();
    }
}
