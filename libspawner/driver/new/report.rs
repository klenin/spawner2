use crate::{Error, Result};
use driver::new::mb2b;
use driver::new::opts::{Options, StdioRedirectList};
use json::{array, object, JsonValue};
use runner::{ExitStatus, RunnerReport, TerminationReason};
use std::fmt::{self, Display, Formatter};
use std::time::Duration;

pub struct Report {
    pub runner_reports: Result<Vec<RunnerReport>>,
    pub cmds: Vec<Options>,
}

pub enum CommandReportKind {
    Json(JsonValue),
    Legacy(String),
}

pub struct CommandReport<'a> {
    pub runner_report: std::result::Result<&'a RunnerReport, &'a Error>,
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
        self.write_legacy(&mut s).unwrap();
        s
    }

    pub fn write_legacy<W: fmt::Write>(&self, w: &mut W) -> fmt::Result {
        fn secs_or_inf<W: fmt::Write>(
            w: &mut W,
            prefix: &'static str,
            val: &Option<Duration>,
        ) -> fmt::Result {
            match val {
                Some(v) => write!(w, "{}{:.6} (sec)\n", prefix, dur2sec(v)),
                None => write!(w, "{}Infinity\n", prefix),
            }
        }
        fn mb_or_inf<W: fmt::Write>(
            w: &mut W,
            prefix: &'static str,
            val: &Option<f64>,
        ) -> fmt::Result {
            match val {
                Some(v) => write!(w, "{}{:.6} (Mb)\n", prefix, v),
                None => write!(w, "{}Infinity\n", prefix),
            }
        }

        let params = if self.cmd.argv.len() == 1 {
            "<none>".to_string()
        } else {
            self.cmd.argv[1..].join(" ")
        };
        write!(w, "\n")?;
        write!(w, "--------------- Spawner report ---------------\n")?;
        write!(w, "Application:               {}\n", self.cmd.argv[0])?;
        write!(w, "Parameters:                {}\n", params)?;
        write!(w, "SecurityLevel:             {}\n", self.cmd.secure as u32)?;
        write!(w, "CreateProcessMethod:       CreateProcess\n")?;
        write!(w, "UserName:                  \n")?;

        secs_or_inf(w, "UserTimeLimit:             ", &self.cmd.time_limit)?;
        secs_or_inf(
            w,
            "DeadLine:                  ",
            &self.cmd.wall_clock_time_limit,
        )?;
        mb_or_inf(w, "MemoryLimit:               ", &self.cmd.memory_limit)?;
        mb_or_inf(w, "WriteLimit:                ", &self.cmd.write_limit)?;
        write!(w, "----------------------------------------------\n")?;

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
        write!(w, "UserTime:                  {:.6} (sec)\n", user_time)?;
        write!(w, "PeakMemoryUsed:            {:.6} (Mb)\n", mem_used)?;
        write!(w, "Written:                   {:.6} (Mb)\n", written)?;
        write!(w, "TerminateReason:           {}\n", term_reason)?;
        write!(w, "ExitCode:                  {}\n", exit_code)?;
        write!(w, "ExitStatus:                {}\n", exit_status)?;
        write!(w, "----------------------------------------------\n")?;
        write!(w, "SpawnerError:              {}\n", error)?;
        Ok(())
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

impl Display for CommandReportKind {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            CommandReportKind::Json(v) => write!(f, "{:#}", v),
            CommandReportKind::Legacy(s) => s.fmt(f),
        }
    }
}

impl<'a> Display for CommandReport<'a> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if self.cmd.use_json {
            write!(f, "{:#}", self.to_json())
        } else {
            self.write_legacy(f)
        }
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
