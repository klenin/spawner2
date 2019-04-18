use crate::iograph::{IoBuilder, IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId};
use crate::pipe::{ReadPipe, WritePipe};
use crate::runner::{Process, ProcessStdio, RunnerController, RunnerReport, RunnerThread};
use crate::rwhub::{ReadHubController, ReadHubThread, WriteHub};
use crate::{Error, Result};

use std::fmt;
use std::time::Duration;
use std::u64;

#[derive(Copy, Clone, Debug)]
pub struct ResourceLimits {
    /// The maximum allowed amount of time for a command.
    pub max_wall_clock_time: Option<Duration>,
    /// Idle time is wall clock time - user time.
    pub max_idle_time: Option<Duration>,
    /// The maximum allowed amount of user-mode execution time for a command.
    pub max_user_time: Option<Duration>,
    /// The maximum allowed memory usage, in bytes.
    pub max_memory_usage: Option<u64>,
    /// The maximum allowed amount of bytes written by a command.
    pub max_output_size: Option<u64>,
    /// The maximum allowed number of processes created.
    pub max_processes: Option<usize>,
}

#[derive(Copy, Clone, Debug)]
pub enum EnvKind {
    Clear,
    Inherit,
    UserDefault,
}

pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

pub struct Task {
    pub app: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub show_window: bool,
    pub create_suspended: bool,
    pub limits: ResourceLimits,
    pub monitor_interval: Duration,
    pub env_kind: EnvKind,
    pub env_vars: Vec<(String, String)>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub on_terminate: Option<Box<OnTerminate>>,
    pub stdout_controller: Option<Box<ReadHubController>>,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: OstreamId,
    pub stdout: IstreamId,
    pub stderr: IstreamId,
}

pub struct Tasks {
    tasks: Vec<(Task, StdioMapping)>,
    io: IoBuilder,
}

#[derive(Debug)]
pub struct TaskErrors {
    pub errors: Vec<Error>,
}

pub type TaskResult = std::result::Result<RunnerReport, TaskErrors>;

struct TaskThreads {
    runner: RunnerThread,
    stdio: StdioThreads,
}

struct StdioThreads {
    stdout: Option<ReadHubThread>,
    stderr: Option<ReadHubThread>,
}

#[derive(Clone)]
pub struct TaskController {
    runner_ctl: RunnerController,
}

pub struct Spawner {
    tasks: Vec<TaskThreads>,
    input_files: Vec<ReadHubThread>,
    output_files: Vec<WriteHub>,
    iograph: IoGraph,
}

impl ResourceLimits {
    pub fn none() -> Self {
        Self {
            max_wall_clock_time: None,
            max_idle_time: None,
            max_user_time: None,
            max_memory_usage: None,
            max_output_size: None,
            max_processes: None,
        }
    }
}

impl Tasks {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            io: IoBuilder::new(),
        }
    }

    pub fn io(&mut self) -> &mut IoBuilder {
        &mut self.io
    }

    pub fn add<T: Into<Task>>(&mut self, t: T) -> Result<StdioMapping> {
        let mut task = t.into();
        let mapping = StdioMapping {
            stdin: self.io.add_ostream(None)?,
            stdout: self.io.add_istream(None, task.stdout_controller.take())?,
            stderr: self.io.add_istream(None, None)?,
        };
        self.tasks.push((task, mapping));
        Ok(mapping)
    }

    pub fn extend<T, U>(&mut self, items: T) -> Result<()>
    where
        T: IntoIterator<Item = U>,
        U: Into<Task>,
    {
        for task in items.into_iter() {
            self.add(task.into())?;
        }
        Ok(())
    }

    pub fn stdio_mapping(&self, i: usize) -> StdioMapping {
        self.tasks[i].1
    }

    pub fn stdio_mappings<'a>(&'a self) -> impl Iterator<Item = StdioMapping> + 'a {
        self.tasks.iter().map(|(_, m)| m.clone())
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

impl TaskController {
    pub fn runner_controller(&self) -> &RunnerController {
        &self.runner_ctl
    }
}

impl Spawner {
    pub fn spawn(tasks: Tasks) -> Result<Self> {
        let (mut iostreams, iograph) = tasks.io.build();

        let ps_and_stdio_threads = tasks
            .tasks
            .iter()
            .map(|(task, mapping)| {
                build_stdio(&mapping, &mut iostreams, &iograph).and_then(
                    |(ps_stdio, stdio_threads)| {
                        Ok((Process::suspended(&task, ps_stdio)?, stdio_threads))
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

        let tasks = tasks
            .tasks
            .into_iter()
            .zip(ps_and_stdio_threads.into_iter())
            .map(|((mut task, _), (ps, stdio))| {
                let on_terminate = task.on_terminate.take();
                RunnerThread::spawn(task, ps, on_terminate).map(|rt| TaskThreads {
                    runner: rt,
                    stdio: stdio,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Spawner {
            tasks: tasks,
            input_files: input_files,
            output_files: output_files,
            iograph: iograph,
        })
    }

    pub fn wait(self) -> Vec<TaskResult> {
        let (runner_threads, stdio_threads): (Vec<RunnerThread>, Vec<StdioThreads>) =
            self.tasks.into_iter().map(|t| (t.runner, t.stdio)).unzip();

        let mut results: Vec<TaskResult> = runner_threads
            .into_iter()
            .map(|thread| thread.join().map_err(|e| TaskErrors { errors: vec![e] }))
            .collect();

        drop(self.output_files);
        for reader in self.input_files.into_iter() {
            let _ = reader.join();
        }
        for (stdio, result) in stdio_threads.into_iter().zip(results.iter_mut()) {
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

    pub fn controllers<'a>(&'a self) -> impl Iterator<Item = TaskController> + 'a {
        self.tasks.iter().map(|t| TaskController {
            runner_ctl: t.runner.controller(),
        })
    }

    pub fn io_graph(&self) -> &IoGraph {
        &self.iograph
    }
}

fn build_stdio(
    mapping: &StdioMapping,
    iostreams: &mut IoStreams,
    iograph: &IoGraph,
) -> Result<(ProcessStdio, StdioThreads)> {
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
        StdioThreads {
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
