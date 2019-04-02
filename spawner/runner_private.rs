use crate::command::{Command, OnTerminate};
use crate::pipe::{ReadPipe, WritePipe};
use crate::runner::{Runner, RunnerReport};
use crate::sys::runner as runner_impl;
use crate::sys::IntoInner;
use crate::{Error, Result};

use std::thread::{self, JoinHandle};

pub struct RunnerThread {
    handle: JoinHandle<Result<RunnerReport>>,
    runner: Runner,
}

pub struct MonitoringLoop(runner_impl::MonitoringLoop);

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

impl RunnerThread {
    pub fn spawn(ml: MonitoringLoop, mut on_terminate: Option<Box<OnTerminate>>) -> Result<Self> {
        let runner = ml.runner();
        thread::Builder::new()
            .spawn(move || {
                let result = ml.entry();
                if let Some(handler) = on_terminate.as_mut() {
                    handler.on_terminate();
                }
                result
            })
            .map_err(|e| Error::from(e))
            .map(|handle| RunnerThread {
                handle: handle,
                runner: runner,
            })
    }

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn join(self) -> Result<RunnerReport> {
        self.handle
            .join()
            .unwrap_or(Err(Error::from("Runner thread panicked")))
    }
}

impl MonitoringLoop {
    pub fn create(cmd: Command, stdio: ProcessStdio) -> Result<Self> {
        runner_impl::MonitoringLoop::create(
            cmd,
            runner_impl::ProcessStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )
        .map(|ml| Self(ml))
    }

    fn runner(&self) -> Runner {
        self.0.runner()
    }

    fn entry(self) -> Result<RunnerReport> {
        self.0.entry()
    }
}
