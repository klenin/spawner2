extern crate sp_derive;
extern crate spawner;

mod instance;
mod opts;
mod value_parser;

#[cfg(test)]
mod tests;

use instance::SpawnerOptions;
use opts::CmdLineOptions;
use spawner::command::{Command, Limits};
use spawner::runner::{Report, TerminationReason};
use spawner::Spawner;
use std::env;
use std::io;
use std::time::Duration;
use std::u64;

fn parse_argv(argv: &[String]) -> Result<Vec<SpawnerOptions>, String> {
    let mut default_opts = SpawnerOptions::default();
    let mut pos = 0;
    let mut result: Vec<SpawnerOptions> = Vec::new();

    while pos < argv.len() {
        let mut opts = default_opts.clone();
        let num_opts = match opts.parse(&argv[pos..]) {
            Ok(n) => n,
            Err(e) => return Err(e),
        };
        pos += num_opts;

        let mut sep_pos = argv.len();
        if let Some(sep) = &opts.separator {
            let full_sep = format!("--{}", sep);
            if let Some(i) = argv[pos..].iter().position(|x| x == &full_sep) {
                sep_pos = pos + i;
            }
        }
        opts.argv.extend_from_slice(&argv[pos..sep_pos]);
        pos = sep_pos + 1;

        if opts.argv.is_empty() {
            default_opts = opts;
        } else {
            default_opts.separator = opts.separator.clone();
            result.push(opts);
        }
    }
    Ok(result)
}

fn mb2b(mb: f64) -> u64 {
    if mb.is_infinite() {
        u64::MAX
    } else {
        (mb * 1024.0 * 1024.0) as u64
    }
}

impl From<&SpawnerOptions> for Command {
    fn from(opts: &SpawnerOptions) -> Command {
        let mut cmd = Command::new(opts.argv[0].clone());
        cmd.add_args(opts.argv.iter().skip(1))
            .set_limits(Limits {
                max_user_time: opts.time_limit,
                max_memory_usage: mb2b(opts.memory_limit),
                max_output_size: mb2b(opts.write_limit),
                max_processes: opts.process_count as u64,
            })
            .set_monitor_interval(opts.monitor_interval)
            .set_display_gui(!opts.hide_gui);
        cmd
    }
}

fn termination_reason_to_str(r: &TerminationReason) -> &str {
    match r {
        TerminationReason::None => "none",
        TerminationReason::UserTimeLimitExceeded => "user time limit exceeded",
        TerminationReason::WriteLimitExceeded => "write limit exceeded",
        TerminationReason::MemoryLimitExceeded => "memory limit exceeded",
        TerminationReason::Other => "other",
    }
}

fn duration_to_string(dur: &Duration) -> String {
    let usec = dur.as_secs() as f64 * 1e6 + dur.subsec_micros() as f64;
    (usec / 1e6).to_string()
}

fn print_report(report: &Report) {
    let labels = [
        "app",
        "user time",
        "peak memory used",
        "termination reason",
        "exit code",
    ];
    let values = [
        report.cmd.app().to_str().unwrap().to_string(),
        duration_to_string(&report.user_time),
        report.peak_memory_used.to_string(),
        termination_reason_to_str(&report.termination_reason).to_string(),
        report.exit_code.to_string(),
    ];
    let max_label_len = labels.iter().map(|x| x.len()).max().unwrap();

    for (label, val) in labels.iter().zip(values.iter()) {
        println!(
            "{}:{}{}",
            label,
            String::from(" ").repeat(2 + max_label_len - label.len()),
            val
        );
    }
    println!("");
}

fn run(argv: &[String]) -> io::Result<()> {
    let opts =
        parse_argv(argv).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    if opts.len() == 0 {
        println!("{}", SpawnerOptions::help());
        return Ok(());
    }

    let reports = Spawner::spawn(opts.iter().map(|x| Command::from(x)))?.wait()?;
    for report in &reports {
        print_report(report);
    }
    Ok(())
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() == 1 {
        println!("{}", SpawnerOptions::help());
    } else {
        if let Err(e) = run(&args[1..]) {
            println!("{}", e.to_string());
        }
    }
}
