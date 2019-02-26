use std::time::Duration;
use std::u64;
use stdio::IstreamController;

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
    pub current_dir: Option<String>,
    pub show_gui: bool,
    pub spawn_suspended: bool,
    pub limits: Limits,
    pub monitor_interval: Duration,
    pub env_kind: EnvKind,
    pub env_vars: Vec<EnvVar>,
}

pub trait OnTerminate: Send {
    fn on_terminate(&mut self);
}

pub struct CommandController {
    pub on_terminate: Option<Box<OnTerminate>>,
    pub stdout_controller: Option<Box<IstreamController>>,
}

pub struct CommandBuilder {
    cmd: Command,
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
            current_dir: None,
            show_gui: false,
            spawn_suspended: false,
            limits: Limits::none(),
            monitor_interval: Duration::from_millis(1),
            env_kind: EnvKind::Inherit,
            env_vars: Vec::new(),
        }
    }
}

impl CommandBuilder {
    pub fn new<S: AsRef<str>>(app: S) -> Self {
        Self {
            cmd: Command::new(app),
        }
    }

    pub fn arg<S: AsRef<str>>(mut self, arg: S) -> Self {
        self.cmd.args.push(arg.as_ref().to_string());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.cmd
            .args
            .extend(args.into_iter().map(|x| x.as_ref().to_string()));
        self
    }

    pub fn current_dir<S: AsRef<str>>(mut self, dir: S) -> Self {
        self.cmd.current_dir = Some(dir.as_ref().to_string());
        self
    }

    pub fn current_dir_opt<S: AsRef<str>>(mut self, dir: Option<S>) -> Self {
        self.cmd.current_dir = dir.map(|d| d.as_ref().to_string());
        self
    }

    pub fn show_gui(mut self, show: bool) -> Self {
        self.cmd.show_gui = show;
        self
    }

    pub fn spawn_suspended(mut self, suspended: bool) -> Self {
        self.cmd.spawn_suspended = suspended;
        self
    }

    pub fn monitor_interval(mut self, int: Duration) -> Self {
        self.cmd.monitor_interval = int;
        self
    }

    pub fn limits(mut self, l: Limits) -> Self {
        self.cmd.limits = l;
        self
    }

    pub fn max_wall_clock_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_wall_clock_time = Some(t);
        self
    }

    pub fn max_idle_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_idle_time = Some(t);
        self
    }

    pub fn max_user_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_user_time = Some(t);
        self
    }

    pub fn max_memory_usage(mut self, bytes: u64) -> Self {
        self.cmd.limits.max_memory_usage = Some(bytes);
        self
    }

    pub fn max_output_size(mut self, bytes: u64) -> Self {
        self.cmd.limits.max_output_size = Some(bytes);
        self
    }

    pub fn max_processes(mut self, n: usize) -> Self {
        self.cmd.limits.max_processes = Some(n);
        self
    }

    pub fn env_kind(mut self, kind: EnvKind) -> Self {
        self.cmd.env_kind = kind;
        self
    }

    pub fn env_var(mut self, name: String, val: String) -> Self {
        self.cmd.env_vars.push(EnvVar {
            name: name,
            val: val,
        });
        self
    }

    pub fn env_vars<I, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: AsRef<EnvVar>,
    {
        self.cmd
            .env_vars
            .extend(vars.into_iter().map(|v| v.as_ref().clone()));
        self
    }

    pub fn build(self) -> Command {
        self.cmd
    }
}

impl AsRef<EnvVar> for EnvVar {
    fn as_ref(&self) -> &EnvVar {
        self
    }
}
