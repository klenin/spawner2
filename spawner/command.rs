use crate::stdio::IstreamController;

use std::time::Duration;
use std::u64;

#[derive(Copy, Clone, Debug)]
pub struct Limits {
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

#[derive(Clone, Debug)]
pub struct EnvVar {
    pub name: String,
    pub val: String,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub app: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub show_window: bool,
    pub create_suspended: bool,
    pub limits: Limits,
    pub monitor_interval: Duration,
    pub env_kind: EnvKind,
    pub env_vars: Vec<EnvVar>,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

pub struct CommandController {
    pub on_terminate: Option<Box<OnTerminate>>,
    pub stdout_controller: Option<Box<IstreamController>>,
}

impl Limits {
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

impl Command {
    pub fn new<S: AsRef<str>>(app: S) -> Self {
        Self {
            app: app.as_ref().to_string(),
            args: Vec::new(),
            working_directory: None,
            show_window: false,
            create_suspended: false,
            limits: Limits::none(),
            monitor_interval: Duration::from_millis(1),
            env_kind: EnvKind::Inherit,
            env_vars: Vec::new(),
            username: None,
            password: None,
        }
    }
}
