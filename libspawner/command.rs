use std::time::Duration;
use std::u64;

#[derive(Copy, Clone)]
pub struct Limits {
    /// The maximum allowed amount of time for a command.
    pub max_wall_clock_time: Duration,
    /// Idle time is wall clock time - user time.
    pub max_idle_time: Duration,
    /// The maximum allowed amount of user-mode execution time for a command.
    pub max_user_time: Duration,
    /// The maximum allowed memory usage, in bytes.
    pub max_memory_usage: u64,
    /// The maximum allowed amount of bytes written by a command.
    pub max_output_size: u64,
    /// The maximum allowed number of processes created.
    pub max_processes: u64,
}

#[derive(Clone)]
pub struct Command {
    pub app: String,
    pub args: Vec<String>,
    pub current_dir: String,
    pub show_gui: bool,
    pub limits: Limits,
    pub monitor_interval: Duration,
}

pub struct Builder {
    cmd: Command,
}

impl Limits {
    pub fn none() -> Self {
        Self {
            max_wall_clock_time: Duration::from_secs(u64::MAX),
            max_idle_time: Duration::from_secs(u64::MAX),
            max_user_time: Duration::from_secs(u64::MAX),
            max_memory_usage: u64::MAX,
            max_output_size: u64::MAX,
            max_processes: u64::MAX,
        }
    }
}

impl Command {
    pub fn new<S: AsRef<str>>(app: S) -> Self {
        Self {
            app: app.as_ref().to_string(),
            args: Vec::new(),
            current_dir: String::new(),
            show_gui: false,
            limits: Limits::none(),
            monitor_interval: Duration::from_millis(1),
        }
    }
}

impl Builder {
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
        self.cmd.current_dir = dir.as_ref().to_string();
        self
    }

    pub fn show_gui(mut self, show: bool) -> Self {
        self.cmd.show_gui = show;
        self
    }

    pub fn monitor_interval(mut self, int: Duration) -> Self {
        self.cmd.monitor_interval = int;
        self
    }

    pub fn max_wall_clock_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_wall_clock_time = t;
        self
    }

    pub fn max_idle_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_idle_time = t;
        self
    }

    pub fn max_user_time(mut self, t: Duration) -> Self {
        self.cmd.limits.max_user_time = t;
        self
    }

    pub fn max_memory_usage(mut self, bytes: u64) -> Self {
        self.cmd.limits.max_memory_usage = bytes;
        self
    }

    pub fn max_output_size(mut self, bytes: u64) -> Self {
        self.cmd.limits.max_output_size = bytes;
        self
    }

    pub fn max_processes(mut self, n: u64) -> Self {
        self.cmd.limits.max_processes = n;
        self
    }

    pub fn build(self) -> Command {
        self.cmd
    }
}
