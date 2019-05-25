use crate::iograph::{IoBuilder, IoGraph, IoStreams, Istream, IstreamId, Ostream, OstreamId};
use crate::pipe::{ReadPipe, WritePipe};
use crate::process::{GroupRestrictions, ProcessInfo, Stdio};
use crate::runner::{OnTerminate, Runner, RunnerReport, RunnerThread};
use crate::rwhub::{ReadHubController, ReadHubThread, WriteHub};
use crate::{Error, Result};

use std::fmt;
use std::time::Duration;

pub struct Controllers {
    on_terminate: Option<Box<OnTerminate>>,
    stdout_controller: Option<Box<ReadHubController>>,
}

struct Task {
    process_info: ProcessInfo,
    restrictions: GroupRestrictions,
    monitor_interval: Duration,
    on_terminate: Option<Box<OnTerminate>>,
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

/// Describes a collection of tasks along with their standard streams.
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

/// Controls the execution of a task list and all associated entities.
pub struct Spawner {
    tasks: Vec<TaskThreads>,
    input_files: Vec<ReadHubThread>,
    output_files: Vec<WriteHub>,
    iograph: IoGraph,
}

impl Controllers {
    pub fn new() -> Self {
        Self {
            on_terminate: None,
            stdout_controller: None,
        }
    }

    pub fn on_terminate<T>(mut self, on_terminate: T) -> Self
    where
        T: OnTerminate + 'static,
    {
        self.on_terminate = Some(Box::new(on_terminate));
        self
    }

    pub fn stdout_controller<T>(mut self, ctl: T) -> Self
    where
        T: ReadHubController + 'static,
    {
        self.stdout_controller = Some(Box::new(ctl));
        self
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

    pub fn add(
        &mut self,
        process_info: ProcessInfo,
        restrictions: GroupRestrictions,
        monitor_interval: Duration,
        controllers: Controllers,
    ) -> Result<StdioMapping> {
        let mapping = StdioMapping {
            stdin: self.io.add_ostream(None)?,
            stdout: self.io.add_istream(None, controllers.stdout_controller)?,
            stderr: self.io.add_istream(None, None)?,
        };
        self.tasks.push((
            Task {
                process_info: process_info,
                restrictions: restrictions,
                monitor_interval: monitor_interval,
                on_terminate: controllers.on_terminate,
            },
            mapping,
        ));
        Ok(mapping)
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

impl Spawner {
    pub fn spawn(tasks: Tasks) -> Result<Self> {
        let (mut iostreams, iograph) = tasks.io.build();

        let ps_and_stdio_threads = tasks
            .tasks
            .iter()
            .map(|(_, mapping)| build_stdio(&mapping, &mut iostreams, &iograph))
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
            .map(|((task, _), (ps_stdio, stdio_threads))| {
                RunnerThread::spawn(
                    task.process_info,
                    ps_stdio,
                    task.restrictions,
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

    pub fn runners<'a>(&'a self) -> impl Iterator<Item = Runner> + 'a {
        self.tasks.iter().map(|t| t.runner_thread.runner())
    }

    pub fn io_graph(&self) -> &IoGraph {
        &self.iograph
    }
}

fn build_stdio(
    mapping: &StdioMapping,
    iostreams: &mut IoStreams,
    iograph: &IoGraph,
) -> Result<(Stdio, StdioThreads)> {
    let stdin = iostreams.ostreams.remove(&mapping.stdin).unwrap();
    let stdout = iostreams.istreams.remove(&mapping.stdout).unwrap();
    let stderr = iostreams.istreams.remove(&mapping.stderr).unwrap();

    let (stdin_r, _stdin_w) = ostream_endings(stdin, iograph.ostream_edges(mapping.stdin))?;
    let (stdout_w, stdout_r) = istream_endings(stdout, iograph.istream_edges(mapping.stdout))?;
    let (stderr_w, stderr_r) = istream_endings(stderr, iograph.istream_edges(mapping.stderr))?;

    Ok((
        Stdio {
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
