use crate::command::{Command, CommandController, OnTerminate};
use crate::iograph::{IoBuilder, IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId};
use crate::pipe::{ReadPipe, ShareMode, WritePipe};
use crate::runner::{Process, ProcessStdio, RunnerController, RunnerReport, RunnerThread};
use crate::rwhub::{ReadHubThread, WriteHub};
use crate::{Error, Result};

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct TaskController {
    runner_ctl: RunnerController,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: OstreamId,
    pub stdout: IstreamId,
    pub stderr: IstreamId,
}

struct TaskStdio {
    stdout: Option<ReadHubThread>,
    stderr: Option<ReadHubThread>,
}

struct Task {
    runner: RunnerThread,
    stdio: TaskStdio,
}

pub struct Session {
    tasks: Vec<Task>,
    input_files: Vec<ReadHubThread>,
    output_files: Vec<WriteHub>,
    iograph: IoGraph,
}

enum IstreamDstKind {
    Pipe(WritePipe),
    File(PathBuf, ShareMode),
    Ostream(OstreamId),
}

pub struct IstreamDst {
    kind: IstreamDstKind,
}

enum OstreamSrcKind {
    Pipe(ReadPipe),
    File(PathBuf, ShareMode),
    Istream(IstreamId),
}

pub struct OstreamSrc {
    kind: OstreamSrcKind,
}

struct TaskData {
    cmd: Command,
    on_terminate: Option<Box<OnTerminate>>,
    stdio_mapping: StdioMapping,
}

pub struct SessionBuilder {
    io: IoBuilder,
    tasks: Vec<TaskData>,
    output_files: HashMap<PathBuf, OstreamId>,
}

#[derive(Debug)]
pub struct TaskErrors {
    pub errors: Vec<Error>,
}

pub type WaitResult = std::result::Result<RunnerReport, TaskErrors>;

impl TaskController {
    pub fn runner_controller(&self) -> &RunnerController {
        &self.runner_ctl
    }
}

impl Session {
    pub fn controllers<'a>(&'a self) -> impl Iterator<Item = TaskController> + 'a {
        self.tasks.iter().map(|t| TaskController {
            runner_ctl: t.runner.controller(),
        })
    }

    pub fn io_graph(&self) -> &IoGraph {
        &self.iograph
    }

    pub fn wait(self) -> Vec<WaitResult> {
        let (runner_threads, stdio_list): (Vec<RunnerThread>, Vec<TaskStdio>) =
            self.tasks.into_iter().map(|t| (t.runner, t.stdio)).unzip();

        let mut results: Vec<WaitResult> = runner_threads
            .into_iter()
            .map(|thread| thread.join().map_err(|e| TaskErrors { errors: vec![e] }))
            .collect();

        drop(self.output_files);
        for reader in self.input_files.into_iter() {
            let _ = reader.join();
        }
        for (stdio, result) in stdio_list.into_iter().zip(results.iter_mut()) {
            for thread in std::iter::once(stdio.stdout)
                .chain(Some(stdio.stderr))
                .filter_map(|x| x)
            {
                if let Err(e) = thread.join() {
                    match result {
                        Ok(_) => *result = Err(TaskErrors { errors: vec![e] }),
                        Err(te) => te.errors.push(e),
                    }
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

    pub fn ostream(ostream: OstreamId) -> Self {
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

    pub fn istream(istream: IstreamId) -> Self {
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
            io: IoBuilder::new(),
            tasks: Vec::new(),
            output_files: HashMap::new(),
        }
    }

    pub fn add_task(&mut self, cmd: Command, ctl: CommandController) -> Result<StdioMapping> {
        let mapping = StdioMapping {
            stdin: self.io.add_ostream(None)?,
            stdout: self.io.add_istream(None, ctl.stdout_controller)?,
            stderr: self.io.add_istream(None, None)?,
        };
        self.tasks.push(TaskData {
            cmd: cmd,
            on_terminate: ctl.on_terminate,
            stdio_mapping: mapping,
        });
        Ok(mapping)
    }

    pub fn add_istream_dst(&mut self, istream: IstreamId, dst: IstreamDst) -> Result<()> {
        let ostream = match dst.kind {
            IstreamDstKind::Pipe(p) => self.io.add_ostream(Some(p))?,
            IstreamDstKind::File(file, mode) => match self.output_files.entry(file) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    let pipe = WritePipe::open(e.key(), mode)?;
                    *e.insert(self.io.add_ostream(Some(pipe))?)
                }
            },
            IstreamDstKind::Ostream(i) => i,
        };
        self.io.connect(istream, ostream);
        Ok(())
    }

    pub fn add_ostream_src(&mut self, ostream: OstreamId, src: OstreamSrc) -> Result<()> {
        let istream = match src.kind {
            OstreamSrcKind::Pipe(p) => self.io.add_istream(Some(p), None)?,
            OstreamSrcKind::File(file, mode) => self
                .io
                .add_istream(Some(ReadPipe::open(file, mode)?), None)?,
            OstreamSrcKind::Istream(i) => i,
        };
        self.io.connect(istream, ostream);
        Ok(())
    }

    pub fn spawn(self) -> Result<Session> {
        let (mut iostreams, iograph) = self.io.build();

        let ps_and_task_stdio = self
            .tasks
            .iter()
            .map(|t| {
                build_stdio(&t.stdio_mapping, &mut iostreams, &iograph).and_then(
                    |(ps_stdio, task_stdio)| {
                        Ok((Process::suspended(&t.cmd, ps_stdio)?, task_stdio))
                    },
                )
            })
            .collect::<Result<Vec<_>>>()?;

        let output_files = iostreams.ostreams.into_iter().map(|(_, o)| o.src).collect();
        let input_files = iostreams
            .istreams
            .into_iter()
            .map(|(_, i)| ReadHubThread::spawn(i.dst))
            .collect::<Result<Vec<_>>>()?;

        let tasks = self
            .tasks
            .into_iter()
            .zip(ps_and_task_stdio.into_iter())
            .map(|(task, (ps, stdio))| {
                RunnerThread::spawn(task.cmd, ps, task.on_terminate).map(|rt| Task {
                    runner: rt,
                    stdio: stdio,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Session {
            tasks: tasks,
            input_files: input_files,
            output_files: output_files,
            iograph: iograph,
        })
    }
}

impl std::error::Error for TaskErrors {}

impl fmt::Display for TaskErrors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.errors.iter() {
            write!(f, "{}\n", e)?;
        }
        Ok(())
    }
}

fn build_stdio(
    mapping: &StdioMapping,
    iostreams: &mut IoStreams,
    iograph: &IoGraph,
) -> Result<(ProcessStdio, TaskStdio)> {
    let stdin = iostreams.ostreams.remove(&mapping.stdin).unwrap();
    let stdout = iostreams.istreams.remove(&mapping.stdout).unwrap();
    let stderr = iostreams.istreams.remove(&mapping.stderr).unwrap();

    let (stdin_r, _stdin_w) = ostream_endings(stdin, iograph.ostream_edges(mapping.stdin))?;
    let (stdout_w, stdout_r) = istream_endings(stdout, iograph.istream_edges(mapping.stdout))?;
    let (stderr_w, stderr_r) = istream_endings(stderr, iograph.istream_edges(mapping.stderr))?;

    Ok((
        ProcessStdio {
            stdin: stdin_r,
            stdout: stdout_w,
            stderr: stderr_w,
        },
        TaskStdio {
            stdout: stdout_r,
            stderr: stderr_r,
        },
    ))
}

fn ostream_endings(
    ostream: Ostream,
    edges: &Vec<IstreamId>,
) -> Result<(ReadPipe, Option<WriteHub>)> {
    Ok(if edges.is_empty() {
        (ReadPipe::null()?, None)
    } else {
        (ostream.dst.unwrap(), Some(ostream.src))
    })
}

fn istream_endings(
    istream: Istream,
    edges: &Vec<OstreamId>,
) -> Result<(WritePipe, Option<ReadHubThread>)> {
    Ok(if edges.is_empty() {
        (WritePipe::null()?, None)
    } else {
        (
            istream.src.unwrap(),
            Some(ReadHubThread::spawn(istream.dst)?),
        )
    })
}
