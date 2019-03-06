use crate::misc::{b2mb, dur2sec, mb2b};
use crate::opts::{Options, StdioRedirectList};

use spawner::runner::{ExitStatus, ProcessInfo, RunnerReport, TerminationReason};
use spawner::session::{CommandErrors, CommandResult};

use json::{array, object, JsonValue};

use std::fmt::{self, Display, Formatter};
use std::time::Duration;

#[derive(Debug)]
pub struct Report {
    pub runner_reports: Vec<CommandResult>,
    pub cmds: Vec<Options>,
}

pub enum CommandReportKind {
    Json(JsonValue),
    Legacy(String),
}

pub struct CommandReport<'a> {
    pub runner_report: std::result::Result<&'a RunnerReport, &'a CommandErrors>,
    pub cmd: &'a Options,
}

pub struct CommandReportIterator<'a> {
    report: &'a Report,
    pos: usize,
}

struct ReportValues {
    ps_info: ProcessInfo,
    error: String,
    term_reason: String,
    exit_status: String,
    exit_code: u32,
}

impl Report {
    pub fn at(&self, idx: usize) -> CommandReport {
        CommandReport {
            runner_report: self.runner_reports[idx].as_ref(),
            cmd: &self.cmds[idx],
        }
    }

    pub fn iter(&self) -> CommandReportIterator {
        CommandReportIterator {
            report: self,
            pos: 0,
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
        let values = ReportValues::from(self);
        object! {
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
            "Result" => object! {
                "Time" => dur2sec(&values.ps_info.total_user_time),
                "WallClockTime" => dur2sec(&values.ps_info.wall_clock_time),
                "Memory" => values.ps_info.peak_memory_used,
                "BytesWritten" => values.ps_info.total_bytes_written,
                "KernelTime" =>  dur2sec(&values.ps_info.total_kernel_time),
                "ProcessorLoad" => values.proc_load(),
                "WorkingDirectory" => self.cmd.working_directory
                    .as_ref().map_or(String::new(), |d| d.clone()),
            },
            "UserName" => "",
            "TerminateReason" => values.term_reason,
            "ExitCode" => values.exit_code,
            "ExitStatus" => values.exit_status,
            "SpawnerError" => array![values.error],
        }
    }

    pub fn to_legacy(&self) -> String {
        let mut s = String::new();
        self.write_legacy(&mut s).unwrap();
        s
    }

    pub fn write_legacy<W: fmt::Write>(&self, w: &mut W) -> fmt::Result {
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

        let values = ReportValues::from(self);
        let user_time = dur2sec(&values.ps_info.total_user_time);
        let mem_used = b2mb(values.ps_info.peak_memory_used);
        let written = b2mb(values.ps_info.total_bytes_written);
        write!(w, "UserTime:                  {:.6} (sec)\n", user_time)?;
        write!(w, "PeakMemoryUsed:            {:.6} (Mb)\n", mem_used)?;
        write!(w, "Written:                   {:.6} (Mb)\n", written)?;
        write!(w, "TerminateReason:           {}\n", values.term_reason)?;
        write!(w, "ExitCode:                  {}\n", values.exit_code)?;
        write!(w, "ExitStatus:                {}\n", values.exit_status)?;
        write!(w, "----------------------------------------------\n")?;
        write!(w, "SpawnerError:              {}\n", values.error)?;
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

impl<'a> Iterator for CommandReportIterator<'a> {
    type Item = CommandReport<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.report.cmds.len() {
            let report = Some(self.report.at(self.pos));
            self.pos += 1;
            report
        } else {
            None
        }
    }
}

impl ReportValues {
    fn proc_load(&self) -> f64 {
        // C++ spawner computes processor load as total user time / wall clock time.
        // Making it possible for the processor load to be greater than 1.0
        let wc = dur2sec(&self.ps_info.wall_clock_time);
        if wc <= 1e-8 {
            0.0
        } else {
            dur2sec(&self.ps_info.total_user_time) / wc
        }
    }
}

impl<'a> From<&CommandReport<'a>> for ReportValues {
    fn from(cr: &CommandReport) -> Self {
        let (ps_info, exit_status, error) = match cr.runner_report {
            Ok(report) => (
                report.process_info,
                report.exit_status.clone(),
                "<none>".to_string(),
            ),
            Err(err) => (
                ProcessInfo::zeroed(),
                ExitStatus::Finished(0),
                err.to_string(),
            ),
        };
        let (term_reason, code, status) = match exit_status {
            ExitStatus::Finished(code) => ("ExitProcess".to_string(), code, code.to_string()),
            ExitStatus::Terminated(ref reason) => (
                match reason {
                    TerminationReason::WallClockTimeLimitExceeded => "TimeLimitExceeded",
                    TerminationReason::IdleTimeLimitExceeded => "IdleTimeLimitExceeded",
                    TerminationReason::UserTimeLimitExceeded => "TimeLimitExceeded",
                    TerminationReason::WriteLimitExceeded => "WriteLimitExceeded",
                    TerminationReason::MemoryLimitExceeded => "MemoryLimitExceeded",
                    TerminationReason::ProcessLimitExceeded => "ProcessesCountLimitExceeded",
                    TerminationReason::ManuallyTerminated => "TerminatedByController",
                }
                .to_string(),
                0,
                "0".to_string(),
            ),
            ExitStatus::Crashed(code, cause) => {
                ("AbnormalExitProcess".to_string(), code, cause.to_string())
            }
        };
        Self {
            ps_info: ps_info,
            error: error,
            term_reason: term_reason,
            exit_status: status,
            exit_code: code,
        }
    }
}

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

fn mb_or_inf<W: fmt::Write>(w: &mut W, prefix: &'static str, val: &Option<f64>) -> fmt::Result {
    match val {
        Some(v) => write!(w, "{}{:.6} (Mb)\n", prefix, v),
        None => write!(w, "{}Infinity\n", prefix),
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
