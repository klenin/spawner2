use std::ffi::{OsStr, OsString};
use std::time::Duration;
use std::u64;
use sys::process::StartupInfo;

#[derive(Copy, Clone)]
pub struct Limits {
    /// the maximum allowed amount of user-mode execution time for target
    pub max_user_time: Duration,
    /// the maximum allowed memory usage, in bytes
    pub max_memory_usage: u64,
    /// the maximum allowed amount of bytes written by target
    pub max_output_size: u64,
    /// the maximum allowed number of processes created
    pub max_processes: u64,
}

#[derive(Clone)]
pub struct Command {
    pub(crate) info: StartupInfo,
    pub(crate) limits: Limits,
    pub(crate) monitor_interval: Duration,
}

impl Command {
    pub fn new<S: AsRef<OsStr>>(app: S) -> Self {
        Self {
            info: StartupInfo {
                app: app.as_ref().to_os_string(),
                args: Vec::new(),
                cwd: OsString::new(),
                display_gui: false,
            },
            limits: Limits::none(),
            monitor_interval: Duration::from_millis(1),
        }
    }

    pub fn app(&self) -> &OsStr {
        &self.info.app.as_os_str()
    }

    pub fn add_arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.info.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn add_args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.info
            .args
            .extend(args.into_iter().map(|x| x.as_ref().to_os_string()));
        self
    }

    pub fn set_cwd<S: AsRef<OsStr>>(&mut self, cwd: S) -> &mut Self {
        self.info.cwd = cwd.as_ref().to_os_string();
        self
    }

    pub fn set_display_gui(&mut self, v: bool) -> &mut Self {
        self.info.display_gui = v;
        self
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn limits_mut(&mut self) -> &mut Limits {
        &mut self.limits
    }

    pub fn set_limits(&mut self, l: Limits) -> &mut Self {
        self.limits = l;
        self
    }

    pub fn set_monitor_interval(&mut self, int: Duration) -> &mut Self {
        self.monitor_interval = int;
        self
    }
}

impl Limits {
    pub fn none() -> Self {
        Self {
            max_user_time: Duration::from_secs(u64::MAX),
            max_memory_usage: u64::MAX,
            max_output_size: u64::MAX,
            max_processes: u64::MAX,
        }
    }
}
