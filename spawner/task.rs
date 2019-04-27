use crate::iograph::{IoBuilder, IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId};
use crate::pipe::{ReadPipe, WritePipe};
use crate::process::{Process, ProcessInfo, ProcessStdio};
use crate::runner::{Runner, RunnerReport, RunnerThread};
use crate::rwhub::{ReadHubController, ReadHubThread, WriteHub};
use crate::{Error, Result};

use std::fmt;
use std::time::Duration;

/// An action that is performed when the task terminates.
pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

pub struct Task {
    pub process_info: ProcessInfo,
    pub resume_process: bool,
    pub monitor_interval: Duration,
    pub on_terminate: Option<Box<OnTerminate>>,
    pub stdout_controller: Option<Box<ReadHubController>>,
}

/// Mapping of the task's stdio into corresponding [`IoStreams`].
///
/// [`IoStreams`]: struct.IoStreams.html
#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: OstreamId,
    pub stdout: IstreamId,
    pub stderr: IstreamId,
}

/// A task list builder that is used to create [`Spawner`].
///
/// [`Spawner`]: struct.Spawner.html
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
    runner_thread: RunnerThread,
    stdio_threads: StdioThreads,
}

struct StdioThreads {
    stdout: Option<ReadHubThread>,
    stderr: Option<ReadHubThread>,
}

/// Contains task's controllers.
#[derive(Clone)]
pub struct TaskController {
    runner: Runner,
}

/// Controls the execution of a task list and all associated entities.
pub struct Spawner {
    tasks: Vec<TaskThreads>,
    input_files: Vec<ReadHubThread>,
    output_files: Vec<WriteHub>,
    iograph: IoGraph,
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
    pub fn runner(&self) -> &Runner {
        &self.runner
    }
}

impl Spawner {
    /// Spawns the task list.
    pub fn spawn(tasks: Tasks) -> Result<Self> {
        let (mut iostreams, iograph) = tasks.io.build();

        let ps_and_stdio_threads = tasks
            .tasks
            .iter()
            .map(|(task, mapping)| {
                build_stdio(&mapping, &mut iostreams, &iograph).and_then(
                    |(ps_stdio, stdio_threads)| {
                        Ok((
                            Process::suspended(&task.process_info, ps_stdio)?,
                            stdio_threads,
                        ))
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
            .map(|((task, _), (ps, stdio_threads))| {
                RunnerThread::spawn(
                    ps,
                    task.resume_process,
                    task.monitor_interval,
                    task.on_terminate,
                )
                .map(|rt| TaskThreads {
                    runner_thread: rt,
                    stdio_threads: stdio_threads,
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

    /// Waits for the termination of a task list.
    pub fn wait(self) -> Vec<TaskResult> {
        let (runner_threads, stdio_threads): (Vec<_>, Vec<_>) = self
            .tasks
            .into_iter()
            .map(|t| (t.runner_thread, t.stdio_threads))
            .unzip();

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

    /// Returns an iterator over the task controllers.
    pub fn controllers<'a>(&'a self) -> impl Iterator<Item = TaskController> + 'a {
        self.tasks.iter().map(|t| TaskController {
            runner: t.runner_thread.runner(),
        })
    }

    /// Returns a reference to the io graph.
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
