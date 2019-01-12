use command::Command;
use runner::{self, Report, Runner};
use std::io;

pub struct Spawner {
    runner_handles: Vec<runner::WaitHandle>,
    runners: Vec<Runner>,
}

impl Spawner {
    pub fn runners(&self) -> &Vec<Runner> {
        &self.runners
    }

    pub fn spawn<T>(cmds: T) -> io::Result<Spawner>
    where
        T: IntoIterator<Item = Command>,
    {
        let mut runner_handles: Vec<runner::WaitHandle> = Vec::new();
        for cmd in cmds.into_iter() {
            runner_handles.push(runner::run(cmd)?);
        }
        let runners = runner_handles.iter().map(|x| x.runner().clone()).collect();
        Ok(Spawner {
            runner_handles: runner_handles,
            runners: runners,
        })
    }

    pub fn wait(self) -> io::Result<Vec<Report>> {
        let mut error_msg = String::new();
        let mut reports: Vec<Report> = Vec::new();
        for handle in self.runner_handles.into_iter() {
            match handle.wait() {
                Ok(r) => reports.push(r),
                Err(e) => {
                    error_msg.push_str(e.to_string().as_str());
                    error_msg.push('\n');
                }
            }
        }
        match error_msg.len() {
            0 => Ok(reports),
            _ => Err(io::Error::new(io::ErrorKind::Other, error_msg)),
        }
    }
}
