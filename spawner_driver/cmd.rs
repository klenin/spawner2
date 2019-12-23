use crate::value_parser::{
    DefaultValueParser, FileFlagsParser, MemValueParser, PercentValueParser, StderrRedirectParser,
    StdinRedirectParser, StdoutRedirectParser,
};

use spawner_opts::{CmdLineOptions, OptionValueParser};

use spawner::VERSION;

use std::f64;
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Environment {
    Clear,
    Inherit,
    UserDefault,
}

#[derive(Copy, Clone, Debug)]
pub struct RedirectFlags {
    pub flush: bool,
    pub exclusive: bool,
}

#[derive(Clone, Debug)]
pub enum RedirectKind {
    File(String),
    Null,
    Std,
    Stdout(usize),
    Stdin(usize),
    Stderr(usize),
}

#[derive(Clone, Debug)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub flags: RedirectFlags,
}

#[derive(Clone, Debug)]
pub struct RedirectList {
    pub items: Vec<Redirect>,
    pub default_flags: RedirectFlags,
}

pub type StdinRedirectList = RedirectList;
pub type StdoutRedirectList = RedirectList;
pub type StderrRedirectList = RedirectList;

#[derive(CmdLineOptions, Clone, Debug)]
#[optcont(
    delimeters = "=:",
    usage = "sp [options] executable [arguments]",
    default_parser = "DefaultValueParser"
)]
pub struct Command {
    #[opt(
        name = "-tl",
        env = "SP_TIME_LIMIT",
        desc = "Set the time limit for an executable (user time)",
        value_desc = "<number>[unit]"
    )]
    pub time_limit: Option<Duration>,

    #[opt(
        name = "-d",
        env = "SP_DEADLINE",
        desc = "Set the time limit for an executable (wall-clock time)",
        value_desc = "<number>[unit]"
    )]
    pub wall_clock_time_limit: Option<Duration>,

    #[opt(
        name = "-y",
        env = "SP_IDLE_TIME_LIMIT",
        desc = "Set the idle time limit for an executable",
        value_desc = "<number>[unit]"
    )]
    pub idle_time_limit: Option<Duration>,

    #[opt(
        name = "-ml",
        env = "SP_MEMORY_LIMIT",
        desc = "Set the memory limit for an executable",
        value_desc = "<number>[unit]",
        parser = "MemValueParser"
    )]
    pub memory_limit: Option<f64>,

    #[opt(
        name = "-wl",
        env = "SP_WRITE_LIMIT",
        desc = "Set the write limit for an executable",
        value_desc = "<number>[unit]",
        parser = "MemValueParser"
    )]
    pub write_limit: Option<f64>,

    #[opt(
        name = "-lr",
        env = "SP_LOAD_RATIO",
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
        name = "-active-process-count",
        desc = "The maximum allowed number of active processes",
        value_desc = "<number>[unit]"
    )]
    pub active_process_count: Option<usize>,

    #[opt(
        name = "-active-connection-count",
        desc = "The maximum allowed number of internet connections",
        value_desc = "<number>[unit]"
    )]
    pub active_connection_count: Option<usize>,

    #[opt(
        names("-mi", "--monitorInterval"),
        env = "SP_MONITOR_INTERVAL",
        desc = "The sleep interval for a monitoring thread (default: 0.001s)",
        value_desc = "<number>[unit]"
    )]
    pub monitor_interval: Duration,

    #[opt(
        name = "-s",
        env = "SP_SECURITY_LEVEL",
        desc = "Set the security level to 0 or 1",
        value_desc = "{0|1}"
    )]
    pub secure: bool,

    #[opt(
        name = "-sw",
        env = "SP_SHOW_WINDOW",
        desc = "Display program window on the screen",
        value_desc = "{0|1}"
    )]
    pub show_window: bool,

    #[opt(
        name = "--debug",
        env = "SP_DEBUG",
        desc = "Print the stack trace of an error",
        value_desc = "{0|1}"
    )]
    pub debug: bool,

    #[opt(
        name = "-wd",
        env = "SP_DIRECTORY",
        desc = "Set the working directory",
        value_desc = "<dir>"
    )]
    pub working_directory: Option<String>,

    #[opt(
        name = "-hr",
        env = "SP_HIDE_REPORT",
        desc = "Do not display report on console",
        value_desc = "{0|1}"
    )]
    pub hide_report: bool,

    #[opt(
        name = "-ho",
        env = "SP_HIDE_OUTPUT",
        desc = "Do not display output on console",
        value_desc = "{0|1}"
    )]
    pub hide_output: bool,

    #[opt(
        names("-runas", "--delegated"),
        env = "SP_RUNAS",
        desc = "Run spawner as delegate",
        value_desc = "{0|1}"
    )]
    pub delegated: bool,

    #[opt(
        name = "-u",
        env = "SP_USER",
        desc = "Run executable under <user>",
        value_desc = "<user>"
    )]
    pub username: Option<String>,

    #[opt(
        name = "-p",
        env = "SP_PASSWORD",
        desc = "Password for <user>",
        value_desc = "<password>"
    )]
    pub password: Option<String>,

    #[flag(
        names("-c", "--systempath"),
        env = "SP_SYSTEM_PATH",
        desc = "Search for an executable in system path"
    )]
    pub use_syspath: bool,

    #[opt(
        name = "-sr",
        env = "SP_REPORT_FILE",
        desc = "Save report to <file>",
        value_desc = "<file>"
    )]
    pub output_file: Option<String>,

    #[opt(
        name = "-env",
        env = "SP_ENVIRONMENT",
        desc = "Set environment variables for an executable (default: inherit)",
        value_desc = "{inherit|user-default|clear}"
    )]
    pub env: Environment,

    #[opt(
        name = "-D",
        desc = "Define an additional environment variable for an executable",
        value_desc = "<var>"
    )]
    pub env_vars: Vec<(String, String)>,

    #[opt(
        names("-i", "--in"),
        env = "SP_INPUT_FILE",
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
        env = "SP_OUTPUT_FILE",
        desc = "Redirect stdout to [*[<file-flags>]:]<filename>\n\
                or *[[<pipe-flags>]:]{null|std|<index>.stdin}",
        value_desc = "<value>",
        parser = "StdoutRedirectParser"
    )]
    pub stdout_redirect: StdoutRedirectList,

    #[opt(
        names("-e", "-se", "--err"),
        env = "SP_ERROR_FILE",
        desc = "Redirect stderr to [*[<file-flags>]:]<filename>\n\
                or *[[<pipe-flags>]:]{null|std|<index>.stdin}",
        value_desc = "<value>",
        parser = "StderrRedirectParser"
    )]
    pub stderr_redirect: StderrRedirectList,

    #[opt(
        name = "--separator",
        env = "SP_SEPARATOR",
        desc = "Use '--<sep>' to separate executables",
        value_desc = "<sep>"
    )]
    pub separator: Option<String>,

    #[flag(name = "--controller", desc = "Mark an executable as controller")]
    pub controller: bool,

    #[opt(
        name = "--shared-memory",
        env = "SP_SHARED_MEMORY",
        value_desc = "<value>"
    )]
    pub shared_memory: Option<String>,

    #[flag(
        names("-j", "--json"),
        env = "SP_JSON",
        desc = "Use JSON format in report"
    )]
    pub use_json: bool,

    #[flag(
        name = "--wait-for-children",
        desc = "Wait for all child processes to exit"
    )]
    pub wait_for_children: bool,

    pub argv: Vec<String>,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            time_limit: None,
            wall_clock_time_limit: None,
            idle_time_limit: None,
            memory_limit: None,
            write_limit: None,
            load_ratio: 5.0,
            process_count: None,
            active_process_count: None,
            active_connection_count: None,
            monitor_interval: Duration::from_millis(1),
            secure: false,
            show_window: false,
            debug: false,
            working_directory: None,
            hide_report: false,
            hide_output: false,
            delegated: false,
            username: None,
            password: None,
            use_syspath: false,
            output_file: None,
            env: Environment::Inherit,
            env_vars: Vec::new(),
            stdin_redirect: RedirectList::default(),
            stdout_redirect: RedirectList::default(),
            stderr_redirect: RedirectList::default(),
            separator: None,
            controller: false,
            shared_memory: None,
            use_json: false,
            wait_for_children: false,
            argv: Vec::new(),
        }
    }
}

impl Command {
    pub const DEFAULT_FILE_FLAGS: RedirectFlags = RedirectFlags {
        flush: false,
        exclusive: false,
    };

    pub const DEFAULT_PIPE_FLAGS: RedirectFlags = RedirectFlags {
        flush: true,
        exclusive: false,
    };

    pub fn from_env() -> Result<Self, String> {
        let mut opts = Self::default();
        opts.parse_env()?;
        Ok(opts)
    }

    pub fn print_help() {
        let mut help = Self::help();
        help.overview = Some(format!("Spawner sandbox v{}", VERSION));
        println!("{}", help);
        Self::print_redirect_examples();
    }

    fn print_redirect_examples() {
        let examples = [
            ("--in=file.txt", "Redirect file.txt to stdin"),
            ("--in=*:", "Reset default file flags for stdin"),
            ("--out=*e:", "Set default file flags for stdout"),
            (
                "--in=*:file.txt",
                "Redirect file.txt to stdin with default file flags",
            ),
            ("*e:file.txt", "Open file exclusively"),
            (
                "--in=*2.stdout",
                "Redirect stdout of the 2nd command to stdin",
            ),
            ("--out=*null", "Redirect stdout to null"),
            ("--out=*std", "Redirect stdout to the spawner's stdin"),
        ];
        println!("Redirect examples:");
        for (sample, desc) in examples.iter() {
            let indent = "  ";
            let spaces = 30 - (sample.len() + indent.len());
            println!("{}{}{:4$}{}", indent, sample, " ", desc, spaces);
        }
        println!();
    }
}

impl Default for RedirectList {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            default_flags: Command::DEFAULT_FILE_FLAGS,
        }
    }
}

impl Display for RedirectFlags {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}{}",
            if self.flush { "f" } else { "-f" },
            if self.exclusive { "e" } else { "-e" }
        )
    }
}

impl Display for RedirectKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            RedirectKind::File(filename) => write!(f, "{}", filename),
            RedirectKind::Null => write!(f, "null"),
            RedirectKind::Std => write!(f, "std"),
            RedirectKind::Stdout(i) => write!(f, "{}.stdout", i),
            RedirectKind::Stdin(i) => write!(f, "{}.stdin", i),
            RedirectKind::Stderr(i) => write!(f, "{}.stderr", i),
        }
    }
}

impl Display for Redirect {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "*{}:{}", self.flags, self.kind)
    }
}
