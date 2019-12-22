use crate::cmd::{Command, RedirectList};
use crate::driver::DriverResult;
use crate::misc::{b2mb, mb2b};

use spawner::process::ExitStatus;
use spawner::{self, Error, TerminationReason};

use json::{array, object, JsonValue};

use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub struct Report {
    pub application: String,
    pub arguments: Vec<String>,
    pub kind: ReportKind,
    pub limit: ReportLimit,
    pub options: ReportOptions,
    pub working_directory: Option<String>,
    pub create_process_method: String,
    pub username: Option<String>,
    pub stdin: Vec<String>,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
    pub result: ReportResult,
    pub terminate_reason: TerminateReason,
    pub exit_code: u32,
    pub exit_status: String,
    pub spawner_error: Vec<Error>,
}

#[derive(Debug, PartialEq)]
pub enum ReportKind {
    Json,
    Legacy,
}

#[derive(Debug)]
pub struct ReportOptions {
    pub search_in_path: bool,
    pub debug: bool,
}

#[derive(Default, Debug)]
pub struct ReportResult {
    pub time: f64,
    pub wall_clock_time: f64,
    pub memory: u64,
    pub bytes_written: u64,
    pub kernel_time: f64,
    pub processor_load: f64,
    pub processes_created: u64,
}

#[derive(Debug)]
pub struct ReportLimit {
    pub time: Option<f64>,
    pub wall_clock_time: Option<f64>,
    pub memory: Option<u64>,
    pub security_level: Option<u32>,
    pub io_bytes: Option<u64>,
    pub idleness_time: Option<f64>,
    pub idleness_processor_load: Option<f64>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TerminateReason {
    ExitProcess,
    AbnormalExitProcess,
    TimeLimitExceeded,
    IdleTimeLimitExceeded,
    WriteLimitExceeded,
    MemoryLimitExceeded,
    ProcessesCountLimitExceeded,
    ActiveProcessesCountLimitExceeded,
    ActiveConnectionCountLimitExceeded,
    TerminatedByController,
}

#[derive(Debug)]
pub struct LegacyReport<'a> {
    pub application: &'a String,
    pub parameters: &'a Vec<String>,
    pub security_level: Option<u32>,
    pub create_process_method: &'a String,
    pub username: &'a Option<String>,
    pub user_time_limit: Option<f64>,
    pub deadline: Option<f64>,
    pub memory_limit: Option<f64>,
    pub write_limit: Option<f64>,
    pub user_time: f64,
    pub peak_memory_used: f64,
    pub written: f64,
    pub terminate_reason: TerminateReason,
    pub exit_code: u32,
    pub exit_status: &'a String,
    pub spawner_error: &'a Vec<Error>,
    pub debug: bool,
}

struct NoneOrJoin<T, U>(T)
where
    T: IntoIterator<Item = U> + Clone,
    U: AsRef<str>;

struct MbOrInf(Option<f64>);
struct FltSecsOrInf(Option<f64>);
struct Mb(f64);
struct FltSecs(f64);

impl Report {
    pub fn new(cmd: &Command, result: DriverResult) -> Self {
        let mut report = Report::from(cmd);
        match result {
            Ok(runner_report) => {
                report.result = ReportResult::from(&runner_report);
                match runner_report.exit_status {
                    ExitStatus::Finished(code) => {
                        report.exit_code = code;
                        report.exit_status = code.to_string();
                    }
                    ExitStatus::Crashed(cause) => {
                        report.terminate_reason = TerminateReason::AbnormalExitProcess;
                        report.exit_status = cause.to_string();
                    }
                }
                if let Some(tr) = runner_report.termination_reason {
                    report.terminate_reason = TerminateReason::from(tr);
                }
            }
            Err(e) => report.spawner_error = e.errors,
        }
        report
    }

    pub fn to_json(&self) -> JsonValue {
        object! {
            "Application" => self.application.clone(),
            "Arguments" => self.arguments.clone(),
            "Limit" => self.limit.to_json(),
            "Options" => object! {
                "SearchInPath" => self.options.search_in_path,
            },
            "WorkingDirectory" => match self.working_directory {
                Some(ref dir) => dir.clone(),
                None => String::new(),
            },
            "CreateProcessMethod" => self.create_process_method.clone(),
            "UserName" => match self.username {
                Some(ref name) => name.clone(),
                None => String::new(),
            },
            "StdIn" => self.stdin.clone(),
            "StdOut" => self.stdout.clone(),
            "StdErr" => self.stderr.clone(),
            "Result" => object! {
                "Time" => self.result.time,
                "WallClockTime" => self.result.wall_clock_time,
                "Memory" => self.result.memory,
                "BytesWritten" => self.result.bytes_written,
                "KernelTime" =>  self.result.kernel_time,
                "ProcessorLoad" => self.result.processor_load,
            },
            "TerminateReason" => self.terminate_reason.to_string(),
            "ExitCode" => self.exit_code,
            "ExitStatus" => self.exit_status.clone(),
            "SpawnerError" => if self.spawner_error.is_empty() {
                array!["<none>"]
            } else {
                self.spawner_error
                    .iter()
                    .map(|e| {
                        if self.options.debug {
                            format!("{:?}", e)
                        } else {
                            format!("{}", e)
                        }
                        .into()
                    })
                    .collect::<Vec<JsonValue>>().into()
            }
        }
    }

    fn as_legacy(&self) -> LegacyReport {
        LegacyReport {
            application: &self.application,
            parameters: &self.arguments,
            security_level: self.limit.security_level,
            create_process_method: &self.create_process_method,
            username: &self.username,
            user_time_limit: self.limit.time,
            deadline: self.limit.wall_clock_time,
            memory_limit: self.limit.memory.map(|b| b2mb(b)),
            write_limit: self.limit.io_bytes.map(|b| b2mb(b)),
            user_time: self.result.time,
            peak_memory_used: b2mb(self.result.memory),
            written: b2mb(self.result.bytes_written),
            terminate_reason: self.terminate_reason,
            exit_code: self.exit_code,
            exit_status: &self.exit_status,
            spawner_error: &self.spawner_error,
            debug: self.options.debug,
        }
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self.kind {
            ReportKind::Json => write!(f, "{:#}", self.to_json()),
            ReportKind::Legacy => write!(f, "{}", self.as_legacy()),
        }
    }
}

impl From<&Command> for Report {
    fn from(cmd: &Command) -> Self {
        assert!(!cmd.argv.is_empty());

        let limit = ReportLimit::from(cmd);
        let mut argv = cmd.argv.iter();
        Self {
            application: argv.next().unwrap().clone(),
            arguments: argv.map(|a| a.clone()).collect(),
            kind: if cmd.use_json {
                ReportKind::Json
            } else {
                ReportKind::Legacy
            },
            limit,
            options: ReportOptions {
                search_in_path: cmd.use_syspath,
                debug: cmd.debug,
            },
            working_directory: cmd.working_directory.clone(),
            create_process_method: "CreateProcess".to_string(),
            username: cmd.username.clone(),
            stdin: Vec::from(&cmd.stdin_redirect),
            stdout: Vec::from(&cmd.stdout_redirect),
            stderr: Vec::from(&cmd.stderr_redirect),
            result: ReportResult::default(),
            terminate_reason: TerminateReason::ExitProcess,
            exit_code: 0,
            exit_status: "0".to_string(),
            spawner_error: Vec::new(),
        }
    }
}

impl ReportKind {
    pub fn is_json(&self) -> bool {
        match self {
            ReportKind::Json => true,
            _ => false,
        }
    }
}

impl From<&spawner::Report> for ReportResult {
    fn from(report: &spawner::Report) -> Self {
        let timers = report.timers.unwrap_or_default();
        let memory = report.memory.unwrap_or_default();
        let io = report.io.unwrap_or_default();
        let pid_counters = report.pid_counters.unwrap_or_default();

        let time = timers.total_user_time.as_secs_f64();
        let wc_time = report.wall_clock_time.as_secs_f64();
        Self {
            time,
            wall_clock_time: wc_time,
            memory: memory.max_usage,
            bytes_written: io.total_bytes_written,
            kernel_time: timers.total_kernel_time.as_secs_f64(),
            processor_load: if wc_time <= 1e-8 { 0.0 } else { time / wc_time },
            processes_created: pid_counters.total_processes as u64,
        }
    }
}

impl ReportLimit {
    fn to_json(&self) -> JsonValue {
        let mut limit = JsonValue::new_object();
        if let Some(t) = self.time {
            limit["Time"] = t.into();
        }
        if let Some(t) = self.wall_clock_time {
            limit["WallClockTime"] = t.into();
        }
        if let Some(v) = self.memory {
            limit["Memory"] = v.into();
        }
        if let Some(lvl) = self.security_level {
            limit["SecurityLevel"] = lvl.into();
        }
        if let Some(b) = self.io_bytes {
            limit["IOBytes"] = b.into();
        }
        if let Some(t) = self.idleness_time {
            limit["IdlenessTime"] = t.into();
        }
        if let Some(v) = self.idleness_processor_load {
            limit["IdlenessProcessorLoad"] = v.into();
        }
        limit
    }
}

impl From<&Command> for ReportLimit {
    fn from(cmd: &Command) -> Self {
        Self {
            time: cmd.time_limit.map(|d| d.as_secs_f64()),
            wall_clock_time: cmd.wall_clock_time_limit.map(|d| d.as_secs_f64()),
            memory: cmd.memory_limit.map(|x| mb2b(x)),
            security_level: match cmd.secure {
                true => Some(1),
                false => None,
            },
            io_bytes: cmd.write_limit.map(|x| mb2b(x)),
            idleness_time: cmd.idle_time_limit.map(|d| d.as_secs_f64()),
            idleness_processor_load: Some(cmd.load_ratio),
        }
    }
}

impl Display for TerminateReason {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str(match self {
            TerminateReason::ExitProcess => "ExitProcess",
            TerminateReason::AbnormalExitProcess => "AbnormalExitProcess",
            TerminateReason::TimeLimitExceeded => "TimeLimitExceeded",
            TerminateReason::IdleTimeLimitExceeded => "IdleTimeLimitExceeded",
            TerminateReason::WriteLimitExceeded => "WriteLimitExceeded",
            TerminateReason::MemoryLimitExceeded => "MemoryLimitExceeded",
            TerminateReason::ProcessesCountLimitExceeded => "ProcessesCountLimitExceeded",
            TerminateReason::ActiveProcessesCountLimitExceeded => {
                "ActiveProcessesCountLimitExceeded"
            }
            TerminateReason::ActiveConnectionCountLimitExceeded => {
                "ActiveConnectionCountLimitExceeded"
            }
            TerminateReason::TerminatedByController => "TerminatedByController",
        })
    }
}

impl From<TerminationReason> for TerminateReason {
    fn from(reason: TerminationReason) -> Self {
        match reason {
            TerminationReason::WallClockTimeLimitExceeded => TerminateReason::TimeLimitExceeded,
            TerminationReason::IdleTimeLimitExceeded => TerminateReason::IdleTimeLimitExceeded,
            TerminationReason::UserTimeLimitExceeded => TerminateReason::TimeLimitExceeded,
            TerminationReason::WriteLimitExceeded => TerminateReason::WriteLimitExceeded,
            TerminationReason::MemoryLimitExceeded => TerminateReason::MemoryLimitExceeded,
            TerminationReason::ProcessLimitExceeded => TerminateReason::ProcessesCountLimitExceeded,
            TerminationReason::ActiveProcessLimitExceeded => {
                TerminateReason::ActiveProcessesCountLimitExceeded
            }
            TerminationReason::ActiveNetworkConnectionLimitExceeded => {
                TerminateReason::ActiveConnectionCountLimitExceeded
            }
            TerminationReason::TerminatedByRunner => TerminateReason::TerminatedByController,
        }
    }
}

macro_rules! line {
    ($f:expr, $name:expr, $val:expr) => {
        write!($f, "{0: <27}{1}\n", $name, $val)
    };
}

impl<'a> Display for LegacyReport<'a> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "\n--------------- Spawner report ---------------\n")?;
        line!(f, "Application:", self.application)?;
        line!(f, "Parameters:", NoneOrJoin(self.parameters.iter()))?;
        line!(f, "SecurityLevel:", self.security_level.unwrap_or(0))?;
        line!(f, "CreateProcessMethod:", self.create_process_method)?;
        line!(
            f,
            "UserName:",
            self.username.as_ref().unwrap_or(&String::new())
        )?;
        line!(f, "UserTimeLimit:", FltSecsOrInf(self.user_time_limit))?;
        line!(f, "DeadLine:", FltSecsOrInf(self.deadline))?;
        line!(f, "MemoryLimit:", MbOrInf(self.memory_limit))?;
        line!(f, "WriteLimit:", MbOrInf(self.write_limit))?;
        write!(f, "----------------------------------------------\n")?;
        line!(f, "UserTime:", FltSecs(self.user_time))?;
        line!(f, "PeakMemoryUsed:", Mb(self.peak_memory_used))?;
        line!(f, "Written:", Mb(self.written))?;
        line!(f, "TerminateReason:", self.terminate_reason)?;
        line!(f, "ExitCode:", self.exit_code)?;
        line!(f, "ExitStatus:", self.exit_status)?;
        write!(f, "----------------------------------------------\n")?;
        line!(
            f,
            "SpawnerError:",
            NoneOrJoin(self.spawner_error.iter().map(|e| if self.debug {
                format!("{:?}", e)
            } else {
                format!("{}", e)
            }))
        )
    }
}

impl From<&RedirectList> for Vec<String> {
    fn from(list: &RedirectList) -> Vec<String> {
        list.items.iter().map(|x| x.to_string()).collect()
    }
}

impl<T, U> Display for NoneOrJoin<T, U>
where
    T: IntoIterator<Item = U> + Clone,
    U: AsRef<str>,
{
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let mut empty = true;
        for i in self.0.clone().into_iter() {
            empty = false;
            write!(f, "{} ", i.as_ref())?;
        }
        if empty {
            f.write_str("<none>")?;
        }
        Ok(())
    }
}

impl Display for Mb {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:.6} (Mb)", self.0)
    }
}

impl Display for FltSecs {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:.6} (sec)", self.0)
    }
}

impl Display for MbOrInf {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{}", Mb(v)),
            None => write!(f, "Infinity"),
        }
    }
}

impl Display for FltSecsOrInf {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{}", FltSecs(v)),
            None => write!(f, "Infinity"),
        }
    }
}
