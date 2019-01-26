use command::Command;
use io::graph::Builder;
use io::split_combine::{Combiner, Splitter, StopHandle};
use pipe::{ReadPipe, WritePipe};
use process::Stdio;
use runner::{Report, Runner};
use runner_private::{run, WaitHandle};
use std::io;

pub struct Session {
    cmds: Vec<Command>,
    cmds_stdio: Vec<CommandStdio>,
    builder: Builder,
}

pub enum IstreamSrc<'a> {
    Pipe(ReadPipe),
    File(&'a str),
    Ostream(usize),
}

pub enum OstreamDst {
    Pipe(WritePipe),
    Istream(usize),
}

pub struct Spawner {
    active_combiners: Vec<StopHandle>,
    active_splitters: Vec<StopHandle>,
    runner_handles: Vec<WaitHandle>,
    runners: Vec<Runner>,
}

#[derive(Copy, Clone)]
pub struct CommandStdio {
    pub stdin: usize,
    pub stdout: usize,
    pub stderr: usize,
}

struct SpawnerStartupInfo {
    cmds: Vec<Command>,
    splitters: Vec<Splitter>,
    combiners: Vec<Combiner>,
    stdio_list: Vec<Stdio>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            cmds: Vec::new(),
            cmds_stdio: Vec::new(),
            builder: Builder::new(),
        }
    }

    pub fn add_cmd(&mut self, cmd: Command) -> CommandStdio {
        let stdio = CommandStdio {
            stdin: self.builder.add_unknown_istream(),
            stdout: self.builder.add_unknown_ostream(),
            stderr: self.builder.add_unknown_ostream(),
        };
        self.cmds_stdio.push(stdio);
        self.cmds.push(cmd);
        stdio
    }

    pub fn connect_istream(&mut self, istream: usize, src: IstreamSrc) -> io::Result<()> {
        let ostream = match src {
            IstreamSrc::Pipe(p) => self.builder.add_file_ostream(p),
            IstreamSrc::File(f) => self.builder.add_file_ostream(ReadPipe::open(f)?),
            IstreamSrc::Ostream(i) => i,
        };

        self.builder.connect(istream, ostream)
    }

    pub fn connect_ostream(&mut self, ostream: usize, dst: OstreamDst) -> io::Result<()> {
        let istream = match dst {
            OstreamDst::Pipe(p) => self.builder.add_file_istream(p),
            OstreamDst::Istream(i) => i,
        };
        self.builder.connect(istream, ostream)
    }

    pub fn spawn(self) -> io::Result<Spawner> {
        let mut startup_info = self.into_startup_info()?;
        let mut sp = Spawner {
            active_combiners: Vec::new(),
            active_splitters: Vec::new(),
            runner_handles: Vec::new(),
            runners: Vec::new(),
        };

        for combiner in startup_info.combiners.drain(..) {
            sp.active_combiners.push(combiner.start()?);
        }
        for splitter in startup_info.splitters.drain(..) {
            sp.active_splitters.push(splitter.start()?);
        }
        for (cmd, stdio) in startup_info
            .cmds
            .drain(..)
            .zip(startup_info.stdio_list.drain(..))
        {
            let handle = run(cmd, stdio)?;
            sp.runners.push(handle.runner().clone());
            sp.runner_handles.push(handle);
        }

        Ok(sp)
    }

    fn into_startup_info(self) -> io::Result<SpawnerStartupInfo> {
        let graph = self.builder.build()?;
        let mut istreams = graph.istreams;
        let mut ostreams = graph.ostreams;

        let mut splitters: Vec<Splitter> = Vec::new();
        let mut combiners: Vec<Combiner> = Vec::new();
        let stdio_list: Vec<Stdio> = self
            .cmds_stdio
            .iter()
            .map(|streams| {
                let (stdin, stdin_combiner) = istreams[streams.stdin].kind.take_pipe();
                let (stdout, stdout_splitter) = ostreams[streams.stdout].kind.take_pipe();
                let (stderr, stderr_splitter) = ostreams[streams.stderr].kind.take_pipe();
                if let Some(c) = stdin_combiner {
                    combiners.push(c);
                }
                if let Some(s) = stdout_splitter {
                    splitters.push(s);
                }
                if let Some(s) = stderr_splitter {
                    splitters.push(s);
                }
                Stdio {
                    stdin: stdin,
                    stdout: stdout,
                    stderr: stderr,
                }
            })
            .collect();

        for istream in istreams.iter_mut() {
            if istream.kind.is_file_combiner() {
                combiners.push(istream.kind.take_file_combiner());
            }
        }

        for ostream in ostreams.iter_mut() {
            if ostream.kind.is_file_splitter() {
                splitters.push(ostream.kind.take_file_splitter());
            }
        }

        Ok(SpawnerStartupInfo {
            cmds: self.cmds,
            splitters: splitters,
            combiners: combiners,
            stdio_list: stdio_list,
        })
    }
}

impl Spawner {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn wait(mut self) -> io::Result<Vec<Report>> {
        self.wait_impl()
    }

    fn wait_impl(&mut self) -> io::Result<Vec<Report>> {
        let mut reports: Vec<Report> = Vec::new();
        for runner in self.runner_handles.drain(..) {
            reports.push(runner.wait()?);
        }
        // It is (almost) impossible to hang on splitter\combiner deinitialization
        // because all pipes (except user-provided ones) are dead at this point.
        for splitter in self.active_splitters.drain(..) {
            splitter.stop()?;
        }
        for receiver in self.active_combiners.drain(..) {
            receiver.stop()?;
        }
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
