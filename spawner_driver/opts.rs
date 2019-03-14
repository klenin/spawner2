use crate::value_parser::{
    DefaultValueParser, FileFlagsParser, MemValueParser, PercentValueParser, StderrRedirectParser,
    StdinRedirectParser, StdoutRedirectParser,
};

use spawner_opts::{CmdLineOptions, OptionValueParser};

use spawner::command::{EnvKind, EnvVar};
use spawner::pipe::ShareMode;
use spawner::VERSION;

use std::env::{self, VarError};
use std::f64;
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

#[derive(Copy, Clone, Debug)]
pub struct RedirectFlags {
    pub flush: bool,
    pub exclusive: bool,
}

#[derive(Clone, Debug)]
pub enum PipeKind {
    Null,
    Std,
    Stdout(usize),
    Stdin(usize),
    Stderr(usize),
}

#[derive(Clone, Debug)]
pub enum StdioRedirectKind {
    File(String),
    Pipe(PipeKind),
}

#[derive(Clone, Debug)]
pub struct StdioRedirect {
    pub kind: StdioRedirectKind,
    pub flags: RedirectFlags,
}

#[derive(Clone, Debug)]
pub struct StdioRedirectList {
    pub items: Vec<StdioRedirect>,
    pub default_flags: RedirectFlags,
}

pub type StdinRedirectList = StdioRedirectList;
pub type StdoutRedirectList = StdioRedirectList;
pub type StderrRedirectList = StdioRedirectList;

#[derive(CmdLineOptions, Clone, Debug)]
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
    pub username: Option<String>,

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
                or *[[<pipe-flags>]:]{null|std|<index>.stdin}",
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
            username: None,
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

macro_rules! parse_env_var {
    ($val:expr, $var:expr, $parser:ident) => {
        match env::var($var) {
            Ok(v) => $parser::parse(&mut $val, v.as_str())?,
            Err(e) => match e {
                VarError::NotPresent => {}
                _ => return Err(format!("Couldn't interpret {}: {}", $var, e)),
            },
        }
    };

    ($val:expr, $var:expr) => {
        parse_env_var!($val, $var, DefaultValueParser)
    };
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

    pub fn from_env() -> Result<Self, String> {
        let mut opts = Self::default();
        parse_env_var!(opts.time_limit, "SP_TIME_LIMIT");
        parse_env_var!(opts.wall_clock_time_limit, "SP_DEADLINE");
        parse_env_var!(opts.idle_time_limit, "SP_IDLE_TIME_LIMIT");
        parse_env_var!(opts.memory_limit, "SP_MEMORY_LIMIT", MemValueParser);
        parse_env_var!(opts.write_limit, "SP_WRITE_LIMIT", MemValueParser);
        parse_env_var!(opts.load_ratio, "SP_LOAD_RATIO", PercentValueParser);
        parse_env_var!(opts.monitor_interval, "SP_MONITOR_INTERVAL");
        parse_env_var!(opts.secure, "SP_SECURITY_LEVEL");
        parse_env_var!(opts.show_window, "SP_SHOW_WINDOW");
        parse_env_var!(opts.debug, "SP_DEBUG");
        parse_env_var!(opts.working_directory, "SP_DIRECTORY");
        parse_env_var!(opts.hide_report, "SP_HIDE_REPORT");
        parse_env_var!(opts.hide_output, "SP_HIDE_OUTPUT");
        parse_env_var!(opts.delegated, "SP_RUNAS");
        parse_env_var!(opts.username, "SP_USER");
        parse_env_var!(opts.password, "SP_PASSWORD");
        parse_env_var!(opts.use_syspath, "SP_SYSTEM_PATH");
        parse_env_var!(opts.output_file, "SP_REPORT_FILE");
        parse_env_var!(opts.env, "SP_ENVIRONMENT");
        parse_env_var!(opts.stdin_redirect, "SP_INPUT_FILE", StdinRedirectParser);
        parse_env_var!(opts.stdout_redirect, "SP_OUTPUT_FILE", StdoutRedirectParser);
        parse_env_var!(opts.stderr_redirect, "SP_ERROR_FILE", StderrRedirectParser);
        parse_env_var!(opts.separator, "SP_SEPARATOR");
        parse_env_var!(opts.shared_memory, "SP_SHARED_MEMORY");
        parse_env_var!(opts.use_json, "SP_JSON");
        Ok(opts)
    }

    pub fn print_help() {
        let mut help = Self::help();
        help.overview = Some(format!("Spawner sandbox v{}", VERSION));
        println!("{}", help);
        Self::print_redirect_examples();
        Self::print_env_help();
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
        println!("");
    }

    fn print_env_help() {
        let env_opts = [
            ("SP_TIME_LIMIT", "-tl"),
            ("SP_DEADLINE", "-d"),
            ("SP_IDLE_TIME_LIMIT", "-y"),
            ("SP_MEMORY_LIMIT", "-ml"),
            ("SP_WRITE_LIMIT", "-wl"),
            ("SP_LOAD_RATIO", "-lr"),
            ("SP_MONITOR_INTERVAL", "-mi"),
            ("SP_SECURITY_LEVEL", "-s"),
            ("SP_SHOW_WINDOW", "-sw"),
            ("SP_DEBUG", "--debug"),
            ("SP_DIRECTORY", "-wd"),
            ("SP_HIDE_REPORT", "-hr"),
            ("SP_HIDE_OUTPUT", "-ho"),
            ("SP_RUNAS", "-runas, --delegated"),
            ("SP_USER", "-u"),
            ("SP_PASSWORD", "-p"),
            ("SP_SYSTEM_PATH", "-c, --systempath"),
            ("SP_REPORT_FILE", "-sr"),
            ("SP_ENVIRONMENT", "-env"),
            ("SP_INPUT_FILE", "-i, --in"),
            ("SP_OUTPUT_FILE", "-so, --out"),
            ("SP_ERROR_FILE", "-e, -se, --err"),
            ("SP_SEPARATOR", "--separator"),
            ("SP_SHARED_MEMORY", "--shared-memory"),
            ("SP_JSON", "--json"),
        ];
        println!("Environment variables and corresponding options:");
        for (var, opt) in env_opts.iter() {
            let indent = "  ";
            let spaces = 30 - (var.len() + indent.len());
            println!("{}{}{:4$}{}", indent, var, " ", opt, spaces);
        }
    }
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

impl Display for PipeKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            PipeKind::Null => write!(f, "null"),
            PipeKind::Std => write!(f, "std"),
            PipeKind::Stdout(i) => write!(f, "{}.stdout", i),
            PipeKind::Stdin(i) => write!(f, "{}.stdin", i),
            PipeKind::Stderr(i) => write!(f, "{}.stderr", i),
        }
    }
}

impl RedirectFlags {
    pub fn share_mode(&self) -> ShareMode {
        if self.exclusive {
            ShareMode::Exclusive
        } else {
            ShareMode::Shared
        }
    }
}

impl Display for RedirectFlags {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}{}",
            match self.flush {
                true => "f",
                false => "-f",
            },
            match self.exclusive {
                true => "e",
                false => "-e",
            }
        )
    }
}

impl Display for StdioRedirectKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            StdioRedirectKind::Pipe(p) => write!(f, "{}", p),
            StdioRedirectKind::File(filename) => write!(f, "{}", filename),
        }
    }
}

impl Display for StdioRedirect {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "*{}:{}", self.flags, self.kind)
    }
}
