use command::{EnvKind, EnvVar};
use driver::new::value_parser::{
    DefaultValueParser, FileFlagsParser, MemValueParser, PercentValueParser, StderrRedirectParser,
    StdinRedirectParser, StdoutRedirectParser,
};
use driver::prelude::{CmdLineOptions, OptionValueParser};
use std::f64;
use std::time::Duration;

#[derive(Copy, Clone)]
pub struct RedirectFlags {
    pub flush: bool,
    pub exclusive: bool,
}

#[derive(Clone)]
pub enum PipeKind {
    Null,
    Std,
    Stdout(usize),
    Stdin(usize),
    Stderr(usize),
}

#[derive(Clone)]
pub enum StdioRedirectKind {
    File(String),
    Pipe(PipeKind),
}

#[derive(Clone)]
pub struct StdioRedirect {
    pub kind: StdioRedirectKind,
    pub flags: RedirectFlags,
}

#[derive(Clone)]
pub struct StdioRedirectList {
    pub items: Vec<StdioRedirect>,
    pub default_flags: RedirectFlags,
}

pub type StdinRedirectList = StdioRedirectList;
pub type StdoutRedirectList = StdioRedirectList;
pub type StderrRedirectList = StdioRedirectList;

#[derive(CmdLineOptions, Clone)]
#[optcont(
    delimeters = "=:",
    usage = "sp [options] executable [arguments]",
    default_parser = "DefaultValueParser"
)]
pub struct Options {
    #[opt(
        name = "-tl",
        desc = "Set the time limit for an executable (user time)",
        value_desc = "<number>[unit]"
    )]
    pub time_limit: Option<Duration>,

    #[opt(
        name = "-d",
        desc = "Set the time limit for an executable (wall-clock time)",
        value_desc = "<number>[unit]"
    )]
    pub wall_clock_time_limit: Option<Duration>,

    #[opt(
        name = "-y",
        desc = "Set the idle time limit for an executable (idle time = wall-clock time - user time)",
        value_desc = "<number>[unit]"
    )]
    pub idle_time_limit: Option<Duration>,

    #[opt(
        name = "-ml",
        desc = "Set the memory limit for an executable",
        value_desc = "<number>[unit]",
        parser = "MemValueParser"
    )]
    pub memory_limit: Option<f64>,

    #[opt(
        name = "-wl",
        desc = "Set the write limit for an executable",
        value_desc = "<number>[unit]",
        parser = "MemValueParser"
    )]
    pub write_limit: Option<f64>,

    #[opt(
        name = "-lr",
        desc = "The required load of the processor for this executable not to be considered idle (default 5%)",
        value_desc = "<number>[%]",
        parser = "PercentValueParser"
    )]
    pub load_ratio: f64,

    #[opt(
        name = "-process-count",
        desc = "The maximum allowed number of processes created",
        value_desc = "<number>[unit]"
    )]
    pub process_count: Option<usize>,

    #[opt(
        names("-mi", "--monitorInterval"),
        desc = "The sleep interval for a monitoring thread (default: 0.001s)",
        value_desc = "<number>[unit]"
    )]
    pub monitor_interval: Duration,

    #[opt(
        name = "-s",
        desc = "Set the security level to 0 or 1",
        value_desc = "{0|1}"
    )]
    pub secure: bool,

    #[opt(
        name = "-sw",
        desc = "Display program window on the screen",
        value_desc = "{0|1}"
    )]
    pub show_window: bool,

    #[opt(name = "--debug", value_desc = "{0|1}")]
    pub debug: bool,

    #[opt(name = "-wd", desc = "Set the working directory", value_desc = "<dir>")]
    pub working_directory: Option<String>,

    #[opt(
        name = "-hr",
        desc = "Do not display report on console",
        value_desc = "{0|1}"
    )]
    pub hide_report: bool,

    #[opt(
        name = "-ho",
        desc = "Do not display output on console",
        value_desc = "{0|1}"
    )]
    pub hide_output: bool,

    #[opt(
        names("-runas", "--delegated"),
        desc = "Run spawner as delegate",
        value_desc = "{0|1}"
    )]
    pub delegated: bool,

    #[opt(
        name = "-u",
        desc = "Run executable under <user>",
        value_desc = "<user>"
    )]
    pub login: Option<String>,

    #[opt(name = "-p", desc = "Password for <user>", value_desc = "<password>")]
    pub password: Option<String>,

    #[flag(
        names("-c", "--systempath"),
        desc = "Search for an executable in system path"
    )]
    pub use_syspath: bool,

    #[opt(name = "-sr", desc = "Save report to <file>", value_desc = "<file>")]
    pub output_file: Option<String>,

    #[opt(
        name = "-env",
        desc = "Set environment variables for an executable (default: inherit)",
        value_desc = "{inherit|user-default|clear}"
    )]
    pub env: EnvKind,

    #[opt(
        name = "-D",
        desc = "Define an additional environment variable for an executable",
        value_desc = "<var>"
    )]
    pub env_vars: Vec<EnvVar>,

    #[opt(
        names("-i", "--in"),
        desc = "Redirect stdin from [*[<file-flags>]:]<filename>\n\
                or *[[<pipe-flags>]:]{null|std|<index>.stdout}",
        value_desc = "<value>",
        parser = "StdinRedirectParser"
    )]
    pub stdin_redirect: StdinRedirectList,

    #[opt(
        names("-ff", "--file-flags"),
        desc = "Set default flags for opened files (f - force flush, e - exclusively open)",
        value_desc = "<flags>",
        parser = "FileFlagsParser"
    )]
    #[opt(
        names("-so", "--out"),
        desc = "Redirect stdout to [*[<file-flags>]:]<filename>\n\
                or *[[<pipe-flags>]:]{null|std|<index>.stdin}",
        value_desc = "<value>",
        parser = "StdoutRedirectParser"
    )]
    pub stdout_redirect: StdoutRedirectList,

    #[opt(
        names("-e", "-se", "--err"),
        desc = "Redirect stderr to [*[<file-flags>]:]<filename>\n\
                or *[[<pipe-flags>]:]{null|std|<index>.stderr}",
        value_desc = "<value>",
        parser = "StderrRedirectParser"
    )]
    pub stderr_redirect: StderrRedirectList,

    #[opt(
        name = "--separator",
        desc = "Use '--<sep>' to separate executables",
        value_desc = "<sep>"
    )]
    pub separator: Option<String>,

    #[flag(name = "--controller", desc = "Mark an executable as controller")]
    pub controller: bool,

    #[opt(name = "--shared-memory", value_desc = "<value>")]
    pub shared_memory: Option<String>,

    #[flag(names("-j", "--json"), desc = "Use JSON format in report")]
    pub use_json: bool,

    pub argv: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            time_limit: None,
            wall_clock_time_limit: None,
            idle_time_limit: None,
            memory_limit: None,
            write_limit: None,
            load_ratio: 5.0,
            process_count: None,
            monitor_interval: Duration::from_millis(1),
            secure: false,
            show_window: false,
            debug: false,
            working_directory: None,
            hide_report: false,
            hide_output: false,
            delegated: false,
            login: None,
            password: None,
            use_syspath: false,
            output_file: None,
            env: EnvKind::Inherit,
            env_vars: Vec::new(),
            stdin_redirect: StdioRedirectList::default(),
            stdout_redirect: StdioRedirectList::default(),
            stderr_redirect: StdioRedirectList::default(),
            separator: None,
            controller: false,
            shared_memory: None,
            use_json: false,
            argv: Vec::new(),
        }
    }
}

impl Options {
    pub const DEFAULT_FILE_FLAGS: RedirectFlags = RedirectFlags {
        flush: false,
        exclusive: false,
    };

    pub const DEFAULT_PIPE_FLAGS: RedirectFlags = RedirectFlags {
        flush: true,
        exclusive: false,
    };
}

impl StdioRedirect {
    pub fn pipe(kind: PipeKind, flags: RedirectFlags) -> Self {
        Self {
            kind: StdioRedirectKind::Pipe(kind),
            flags: flags,
        }
    }

    pub fn file(path: String, flags: RedirectFlags) -> Self {
        Self {
            kind: StdioRedirectKind::File(path),
            flags: flags,
        }
    }
}

impl Default for StdioRedirectList {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            default_flags: Options::DEFAULT_FILE_FLAGS,
        }
    }
}

impl ToString for PipeKind {
    fn to_string(&self) -> String {
        match self {
            PipeKind::Null => String::from("null"),
            PipeKind::Std => String::from("std"),
            PipeKind::Stdout(i) => format!("{}.stdout", i),
            PipeKind::Stdin(i) => format!("{}.stdin", i),
            PipeKind::Stderr(i) => format!("{}.stderr", i),
        }
    }
}

impl ToString for RedirectFlags {
    fn to_string(&self) -> String {
        let f = match self.flush {
            true => "f",
            false => "-f",
        };
        let e = match self.exclusive {
            true => "e",
            false => "-e",
        };
        format!("{}{}", f, e)
    }
}

impl ToString for StdioRedirect {
    fn to_string(&self) -> String {
        let suffix = match &self.kind {
            StdioRedirectKind::Pipe(p) => p.to_string(),
            StdioRedirectKind::File(f) => f.clone(),
        };
        format!("*{}:{}", self.flags.to_string(), suffix)
    }
}
