use crate::{Error, Result};
use driver::new::mb2b;
use driver::new::opts::{Options, StdioRedirectList};
use json::{array, object, stringify_pretty, JsonValue};
use runner::{self, ExitStatus, TerminationReason};
use std::fmt::Write;
use std::time::Duration;

pub struct Report {
    pub runner_reports: Result<Vec<runner::Report>>,
    pub cmds: Vec<Options>,
}

pub enum CommandReportKind {
    Json(JsonValue),
    Legacy(String),
}

pub struct CommandReport<'a> {
    pub runner_report: std::result::Result<&'a runner::Report, &'a Error>,
    pub cmd: &'a Options,
}

impl Report {
    pub fn at(&self, index: usize) -> CommandReport {
        CommandReport {
            runner_report: match &self.runner_reports {
                Ok(reports) => Ok(&reports[index]),
                Err(e) => Err(e),
            },
            cmd: &self.cmds[index],
        }
    }
}

macro_rules! swrite {
    ($string:ident, $($t:tt)*) => {
        let _ = write!(&mut $string, $($t)*);
    };
}

impl<'a> CommandReport<'a> {
    pub fn kind(&self) -> CommandReportKind {
        if self.cmd.use_json {
            CommandReportKind::Json(self.to_json())
        } else {
            CommandReportKind::Legacy(self.to_legacy())
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut obj = object! {
            "Application" => self.cmd.argv[0].clone(),
            "Arguments" => self.cmd.argv[1..].to_vec(),
            "Limit" => json_limits(self.cmd),
            "Options" => object! {
                "SearchInPath" => self.cmd.use_syspath
            },
            "StdIn" => json_redirect_list(&self.cmd.stdin_redirect),
            "StdOut" => json_redirect_list(&self.cmd.stdout_redirect),
            "StdErr" => json_redirect_list(&self.cmd.stderr_redirect),
            "CreateProcessMethod" => "CreateProcess",
        };

        match self.runner_report {
            Ok(report) => {
                obj["Result"] = object! {
                    "Time" => dur2sec(&report.statistics.total_user_time),
                    "WallClockTime" => dur2sec(&report.statistics.wall_clock_time),
                    "Memory" => report.statistics.peak_memory_used,
                    "BytesWritten" => report.statistics.total_bytes_written,
                    "KernelTime" =>  dur2sec(&report.statistics.total_kernel_time),
                    // C++ spawner computes processor load as total user time / wall clock time.
                    // Making it possible for the processor load to be greater than 1.0
                    "ProcessorLoad" =>
                        dur2sec(&report.statistics.total_user_time) / dur2sec(&report.statistics.wall_clock_time),
                    "WorkingDirectory" => report.command.current_dir.clone(),
                };
                obj["UserName"] = "todo".into();
                match report.exit_status {
                    ExitStatus::Finished(code) => {
                        obj["TerminateReason"] = "ExitProcess".into();
                        obj["ExitCode"] = code.into();
                        obj["ExitStatus"] = code.to_string().into();
                    }
                    ExitStatus::Terminated(ref r) => {
                        obj["TerminateReason"] = spawner_term_reason(r).into();
                        obj["ExitCode"] = 0.into();
                        obj["ExitStatus"] = "0".into();
                    }
                }
                obj["SpawnerError"] = array!["<none>"];
            }
            Err(e) => {
                obj["Result"] = object! {
                    "Time" => 0.0,
                    "WallClockTime" => 0.0,
                    "Memory" => 0,
                    "BytesWritten" => 0,
                    "KernelTime" => 0.0,
                    "ProcessorLoad" => 0.0,
                    "WorkingDirectory" => "",
                };
                obj["UserName"] = "".into();
                obj["TerminateReason"] = "ExitProcess".into();
                obj["ExitCode"] = 0.into();
                obj["ExitStatus"] = "0".into();
                obj["SpawnerError"] = array![e.to_string()];
            }
        }
        obj
    }

    pub fn to_legacy(&self) -> String {
        let mut s = String::new();
        let params = if self.cmd.argv.len() == 1 {
            "<none>".to_string()
        } else {
            self.cmd.argv[1..].join(" ")
        };
        swrite!(s, "\n");
        swrite!(s, "--------------- Spawner report ---------------\n");
        swrite!(s, "Application:               {}\n", self.cmd.argv[0]);
        swrite!(s, "Parameters:                {}\n", params);
        swrite!(s, "SecurityLevel:             {}\n", self.cmd.secure as u32);
        swrite!(s, "CreateProcessMethod:       CreateProcess\n");
        swrite!(s, "UserName:                  \n");

        let user_time_limit = fsecs_or_inf(&self.cmd.time_limit);
        let deadline = fsecs_or_inf(&self.cmd.wall_clock_time_limit);
        let mem_limit = mb_or_inf(&self.cmd.memory_limit);
        let write_limit = mb_or_inf(&self.cmd.write_limit);
        swrite!(s, "UserTimeLimit:             {}\n", user_time_limit);
        swrite!(s, "DeadLine:                  {}\n", deadline);
        swrite!(s, "MemoryLimit:               {}\n", mem_limit);
        swrite!(s, "WriteLimit:                {}\n", write_limit);
        swrite!(s, "----------------------------------------------\n");

        let mut user_time = 0.0;
        let mut mem_used = 0.0;
        let mut written = 0.0;
        let mut term_reason = "ExitProcess";
        let mut exit_code = 0;
        let mut exit_status = "0".to_string();
        let mut error = "<none>".to_string();
        match &self.runner_report {
            Ok(report) => {
                user_time = dur2sec(&report.statistics.total_user_time);
                mem_used = b2mb(report.statistics.peak_memory_used);
                written = b2mb(report.statistics.total_bytes_written);
                match report.exit_status {
                    ExitStatus::Finished(code) => {
                        exit_code = code;
                        exit_status = code.to_string();
                    }
                    ExitStatus::Terminated(ref reason) => {
                        term_reason = spawner_term_reason(reason);
                    }
                }
            }
            Err(e) => error = e.to_string(),
        }
        swrite!(s, "UserTime:                  {:.6} (sec)\n", user_time);
        swrite!(s, "PeakMemoryUsed:            {:.6} (Mb)\n", mem_used);
        swrite!(s, "Written:                   {:.6} (Mb)\n", written);
        swrite!(s, "TerminateReason:           {}\n", term_reason);
        swrite!(s, "ExitCode:                  {}\n", exit_code);
        swrite!(s, "ExitStatus:                {}\n", exit_status);
        swrite!(s, "----------------------------------------------\n");
        swrite!(s, "SpawnerError:              {}\n", error);
        s
    }
}

impl CommandReportKind {
    pub fn is_json(&self) -> bool {
        match self {
            CommandReportKind::Json(_) => true,
            _ => false,
        }
    }

    pub fn into_json(self) -> JsonValue {
        match self {
            CommandReportKind::Json(v) => v,
            _ => unreachable!(),
        }
    }
}

impl ToString for CommandReportKind {
    fn to_string(&self) -> String {
        match self {
            CommandReportKind::Json(v) => stringify_pretty(array![v.clone()], 4),
            CommandReportKind::Legacy(s) => s.clone(),
        }
    }
}

impl<'a> ToString for CommandReport<'a> {
    fn to_string(&self) -> String {
        self.kind().to_string()
    }
}

fn fsecs_or_inf(val: &Option<Duration>) -> String {
    match val {
        Some(v) => format!("{:.6} (sec)", dur2sec(v)),
        None => "Infinity".to_string(),
    }
}

fn mb_or_inf(val: &Option<f64>) -> String {
    match val {
        Some(v) => format!("{:.6} (Mb)", v),
        None => "Infinity".to_string(),
    }
}

fn dur2sec(d: &Duration) -> f64 {
    let us = d.as_secs() as f64 * 1e6 + d.subsec_micros() as f64;
    us / 1e6
}

fn b2mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn spawner_term_reason(r: &TerminationReason) -> &'static str {
    match r {
        TerminationReason::WallClockTimeLimitExceeded => "TimeLimitExceeded",
        TerminationReason::IdleTimeLimitExceeded => "IdleTimeLimitExceeded",
        TerminationReason::UserTimeLimitExceeded => "TimeLimitExceeded",
        TerminationReason::WriteLimitExceeded => "WriteLimitExceeded",
        TerminationReason::MemoryLimitExceeded => "MemoryLimitExceeded",
        TerminationReason::ProcessLimitExceeded => "ProcessesCountLimitExceeded",
        TerminationReason::Other => "TerminatedByController",
    }
}

fn json_limits(opts: &Options) -> JsonValue {
    let mut limits = JsonValue::new_object();
    if let Some(v) = &opts.time_limit {
        limits["Time"] = dur2sec(v).into();
    }
    if let Some(v) = &opts.wall_clock_time_limit {
        limits["WallClockTime"] = dur2sec(v).into();
    }
    if let Some(v) = opts.memory_limit {
        limits["Memory"] = mb2b(v).into();
    }
    if opts.secure {
        limits["SecurityLevel"] = 1.into();
    }
    if let Some(v) = opts.write_limit {
        limits["IOBytes"] = mb2b(v).into();
    }
    if let Some(v) = &opts.idle_time_limit {
        limits["IdlenessTime"] = dur2sec(v).into();
    }

    // The "IdlenessProcessorLoad" seems to be always present in c++ spawner.
    limits["IdlenessProcessorLoad"] = opts.load_ratio.into();
    limits
}

fn json_redirect_list(list: &StdioRedirectList) -> JsonValue {
    list.items
        .iter()
        .map(|x| x.to_string().into())
        .collect::<Vec<JsonValue>>()
        .into()
}
