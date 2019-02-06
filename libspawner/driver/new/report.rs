use crate::Result;
use driver::new::mb2b;
use driver::new::opts::{Options, StdioRedirectKind, StdioRedirectList};
use json::{array, object, stringify_pretty, JsonValue};
use runner::{ExitStatus, Report, TerminationReason};
use std::fmt::Write;
use std::time::Duration;

pub enum ReportKind {
    Json(JsonValue),
    Legacy(String),
}

pub fn create(reports: &Result<Vec<Report>>, idx: usize, opts: &Options) -> ReportKind {
    if opts.use_json {
        ReportKind::Json(json_report(reports, idx, opts))
    } else {
        ReportKind::Legacy(legacy_report(reports, idx, opts))
    }
}

impl ReportKind {
    pub fn is_json(&self) -> bool {
        match self {
            ReportKind::Json(_) => true,
            _ => false,
        }
    }

    pub fn into_json(self) -> JsonValue {
        match self {
            ReportKind::Json(v) => v,
            _ => unreachable!(),
        }
    }
}

impl ToString for ReportKind {
    fn to_string(&self) -> String {
        match self {
            ReportKind::Json(v) => stringify_pretty(array![v.clone()], 4),
            ReportKind::Legacy(s) => s.clone(),
        }
    }
}

macro_rules! swrite {
    ($string:ident, $($t:tt)*) => {
        let _ = write!(&mut $string, $($t)*);
    };
}

fn fsecs_or_inf(val: &Option<Duration>) -> String {
    match val {
        Some(v) => format!("{:.6} (sec)", dur_to_fsec(v)),
        None => "Infinity".to_string(),
    }
}

fn mb_or_inf(val: &Option<f64>) -> String {
    match val {
        Some(v) => format!("{:.6} (Mb)", v),
        None => "Infinity".to_string(),
    }
}

fn legacy_report(reports: &Result<Vec<Report>>, idx: usize, opts: &Options) -> String {
    let mut s = String::new();
    let params = if opts.argv.len() == 1 {
        "<none>".to_string()
    } else {
        opts.argv[1..].join(" ")
    };
    swrite!(s, "\n");
    swrite!(s, "--------------- Spawner report ---------------\n");
    swrite!(s, "Application:               {}\n", opts.argv[0]);
    swrite!(s, "Parameters:                {}\n", params);
    swrite!(s, "SecurityLevel:             {}\n", opts.secure as u32);
    swrite!(s, "CreateProcessMethod:       CreateProcess\n");
    swrite!(s, "UserName:                  \n");

    let user_time_limit = fsecs_or_inf(&opts.time_limit);
    let deadline = fsecs_or_inf(&opts.wall_clock_time_limit);
    let mem_limit = mb_or_inf(&opts.memory_limit);
    let write_limit = mb_or_inf(&opts.write_limit);
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
    match reports {
        Ok(reports) => {
            let report = &reports[idx];
            user_time = dur_to_fsec(&report.statistics.total_user_time);
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

fn json_report(reports: &Result<Vec<Report>>, idx: usize, opts: &Options) -> JsonValue {
    let mut obj = object! {
        "Application" => opts.argv[0].clone(),
        "Arguments" => opts.argv[1..].to_vec(),
        "Limit" => json_limits(opts),
        "Options" => object! {
            "SearchInPath" => opts.use_syspath
        },
        "StdIn" => json_redirect_list(&opts.stdin_redirect),
        "StdOut" => json_redirect_list(&opts.stdout_redirect),
        "StdErr" => json_redirect_list(&opts.stderr_redirect),
        "CreateProcessMethod" => "CreateProcess",
    };

    match reports {
        Ok(reports) => {
            let report = &reports[idx];
            obj["Result"] = object! {
                "Time" => dur_to_fsec(&report.statistics.total_user_time),
                "WallClockTime" => dur_to_fsec(&report.statistics.wall_clock_time),
                "Memory" => report.statistics.peak_memory_used,
                "BytesWritten" => report.statistics.total_bytes_written,
                "KernelTime" =>  dur_to_fsec(&report.statistics.total_kernel_time),
                // C++ spawner computes processor load as total user time / wall clock time.
                // Making it possible for the processor load to be greater than 1.0
                "ProcessorLoad" =>
                    dur_to_fsec(&report.statistics.total_user_time) / dur_to_fsec(&report.statistics.wall_clock_time),
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

fn dur_to_fsec(d: &Duration) -> f64 {
    let us = d.as_secs() as f64 * 1e6 + d.subsec_micros() as f64;
    us / 1e6
}

fn json_limits(opts: &Options) -> JsonValue {
    let mut limits = JsonValue::new_object();
    if let Some(v) = &opts.time_limit {
        limits["Time"] = dur_to_fsec(v).into();
    }
    if let Some(v) = &opts.wall_clock_time_limit {
        limits["WallClockTime"] = dur_to_fsec(v).into();
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
        limits["IdlenessTime"] = dur_to_fsec(v).into();
    }

    // The "IdlenessProcessorLoad" seems to be always present in c++ spawner.
    limits["IdlenessProcessorLoad"] = opts.load_ratio.into();
    limits
}

fn json_redirect_list(list: &StdioRedirectList) -> JsonValue {
    list.items
        .iter()
        .map(|x| match &x.kind {
            StdioRedirectKind::Pipe(p) => format!("*{}", p.to_string()),
            StdioRedirectKind::File(f) => f.clone(),
        })
        .collect::<Vec<String>>()
        .into()
}

fn b2mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
