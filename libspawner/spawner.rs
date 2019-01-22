use command::Command;
use internals::io::graph::Builder;
use internals::io::mpsc::{Receiver, Sender, StopHandle};
use internals::runner::{self, WaitHandle};
use pipe::{ReadPipe, WritePipe};
use process::Stdio;
use runner::{Report, Runner};
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
    active_receivers: Vec<StopHandle>,
    active_senders: Vec<StopHandle>,
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
    senders: Vec<Sender<ReadPipe>>,
    receivers: Vec<Receiver<WritePipe>>,
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
            active_receivers: Vec::new(),
            active_senders: Vec::new(),
            runner_handles: Vec::new(),
            runners: Vec::new(),
        };

        for receiver in startup_info.receivers.drain(..) {
            sp.active_receivers.push(receiver.start()?);
        }
        for sender in startup_info.senders.drain(..) {
            sp.active_senders.push(sender.start()?);
        }
        for (cmd, stdio) in startup_info
            .cmds
            .drain(..)
            .zip(startup_info.stdio_list.drain(..))
        {
            let handle = runner::run(cmd, stdio)?;
            sp.runners.push(handle.runner().clone());
            sp.runner_handles.push(handle);
        }

        Ok(sp)
    }

    fn into_startup_info(self) -> io::Result<SpawnerStartupInfo> {
        let graph = self.builder.build()?;
        let mut istreams = graph.istreams;
        let mut ostreams = graph.ostreams;

        let mut senders: Vec<Sender<ReadPipe>> = Vec::new();
        let mut receivers: Vec<Receiver<WritePipe>> = Vec::new();
        let stdio_list: Vec<Stdio> = self
            .cmds_stdio
            .iter()
            .map(|streams| {
                let (stdin, stdin_receiver) = istreams[streams.stdin].kind.take_pipe();
                let (stdout, stdout_sender) = ostreams[streams.stdout].kind.take_pipe();
                let (stderr, stderr_sender) = ostreams[streams.stderr].kind.take_pipe();
                if let Some(r) = stdin_receiver {
                    receivers.push(r);
                }
                if let Some(s) = stdout_sender {
                    senders.push(s);
                }
                if let Some(s) = stderr_sender {
                    senders.push(s);
                }
                Stdio {
                    stdin: stdin,
                    stdout: stdout,
                    stderr: stderr,
                }
            })
            .collect();

        for istream in istreams.iter_mut() {
            if istream.kind.is_file_receiver() {
                receivers.push(istream.kind.take_file_receiver());
            }
        }

        for ostream in ostreams.iter_mut() {
            if ostream.kind.is_file_sender() {
                senders.push(ostream.kind.take_file_sender());
            }
        }

        Ok(SpawnerStartupInfo {
            cmds: self.cmds,
            senders: senders,
            receivers: receivers,
            stdio_list: stdio_list,
        })
    }
}

impl Spawner {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn wait(mut self) -> io::Result<Vec<Report>> {
        let mut reports: Vec<Report> = Vec::new();
        for runner in self.runner_handles.drain(..) {
            reports.push(runner.wait()?);
        }
        for sender in self.active_senders.drain(..) {
            sender.stop()?;
        }
        for receiver in self.active_receivers.drain(..) {
            receiver.stop()?;
        }
        Ok(reports)
    }
}

impl Drop for Spawner {
    fn drop(&mut self) {
        for runner in self.runners.iter() {
            runner.kill();
        }
        for sender in self.active_senders.drain(..) {
            let _ = sender.stop();
        }
        for receiver in self.active_receivers.drain(..) {
            let _ = receiver.stop();
        }
        for handle in self.runner_handles.drain(..) {
            let _ = handle.wait();
        }
    }
}
