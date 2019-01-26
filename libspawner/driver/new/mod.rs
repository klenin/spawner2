pub mod opts;
mod value_parser;

#[cfg(test)]
mod tests;

use command::{self, Command};
use driver::new::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use driver::prelude::*;
use runner::{ExitStatus, Report, TerminationReason};
use session::{CommandStdio, IstreamSrc, OstreamDst, Session};
use std::env;
use std::io;
use std::time::Duration;
use std::u64;

fn parse_argv(argv: Vec<String>) -> io::Result<Vec<Options>> {
    let mut default_opts = Options::default();
    let mut pos = 0;
    let mut result: Vec<Options> = Vec::new();

    while pos < argv.len() {
        let mut opts = default_opts.clone();
        let num_opts = match opts.parse(&argv[pos..]) {
            Ok(n) => n,
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
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

fn redirect_istream(
    sess: &mut Session,
    istream: usize,
    stdio: &Vec<CommandStdio>,
    redirect_list: &StdioRedirectList,
) -> io::Result<()> {
    for redirect in redirect_list.items.iter() {
        match &redirect.kind {
            StdioRedirectKind::File(s) => {
                sess.connect_istream(istream, IstreamSrc::File(s.as_str()))?;
            }
            StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                PipeKind::Null => { /* pipes are null by default */ }
                PipeKind::Std => { /* todo */ }
                PipeKind::Stdout(i) => {
                    // check i
                    sess.connect_istream(istream, IstreamSrc::Ostream(stdio[*i].stdout))?;
                }
                _ => {}
            },
        }
    }
    Ok(())
}

fn redirect_ostream(
    sess: &mut Session,
    ostream: usize,
    stdio: &Vec<CommandStdio>,
    redirect_list: &StdioRedirectList,
) -> io::Result<()> {
    for redirect in redirect_list.items.iter() {
        match &redirect.kind {
            StdioRedirectKind::File(_) => { /* todo */ }
            StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                PipeKind::Null => { /* pipes are null by default */ }
                PipeKind::Std => { /* todo */ }
                PipeKind::Stdin(i) => {
                    // check i
                    sess.connect_ostream(ostream, OstreamDst::Istream(stdio[*i].stdin))?;
                }
                PipeKind::Stderr(_) => {
                    // todo: c++ spawner can redirect stderr to other stderr
                }
                _ => {}
            },
        }
    }
    Ok(())
}

fn mb2b(mb: f64) -> u64 {
    if mb.is_infinite() {
        u64::MAX
    } else {
        (mb * 1024.0 * 1024.0) as u64
    }
}

impl From<&Options> for Command {
    fn from(opts: &Options) -> Command {
        command::Builder::new(opts.argv[0].clone())
            .args(opts.argv.iter().skip(1))
            .max_user_time(opts.time_limit)
            .max_memory_usage(mb2b(opts.memory_limit))
            .max_output_size(mb2b(opts.write_limit))
            .max_processes(opts.process_count as u64)
            .monitor_interval(opts.monitor_interval)
            .show_gui(!opts.hide_gui)
            .build()
    }
}

fn exit_status_to_string(es: &ExitStatus) -> String {
    match es {
        ExitStatus::Normal(c) => c.to_string(),
        ExitStatus::Terminated(r) => match r {
            TerminationReason::UserTimeLimitExceeded => "user time limit exceeded",
            TerminationReason::WriteLimitExceeded => "write limit exceeded",
            TerminationReason::MemoryLimitExceeded => "memory limit exceeded",
            TerminationReason::Other => "other",
        }
        .to_string(),
    }
}

fn duration_to_string(dur: &Duration) -> String {
    let usec = dur.as_secs() as f64 * 1e6 + dur.subsec_micros() as f64;
    (usec / 1e6).to_string()
}

fn print_report(report: &Report) {
    let labels = [
        "app",
        "total user time",
        "total kernel time",
        "peak memory used",
        "total bytes written",
        "total processes",
        "exit status",
    ];
    let values = [
        report.command.app.clone(),
        duration_to_string(&report.statistics.total_user_time),
        duration_to_string(&report.statistics.total_kernel_time),
        report.statistics.peak_memory_used.to_string(),
        report.statistics.total_bytes_written.to_string(),
        report.statistics.total_processes.to_string(),
        exit_status_to_string(&report.exit_status),
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

pub fn run<T, U>(argv: T) -> io::Result<Vec<Report>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let opts = parse_argv(argv.into_iter().map(|x| x.as_ref().to_string()).collect())?;
    if opts.len() == 0 {
        return Ok(Vec::new());
    }

    let mut sess = Session::new();
    let stdio: Vec<CommandStdio> = opts
        .iter()
        .map(|x| sess.add_cmd(Command::from(x)))
        .collect();

    for (opt, opt_stdio) in opts.iter().zip(stdio.iter()) {
        redirect_istream(&mut sess, opt_stdio.stdin, &stdio, &opt.stdin_redirect)?;
        redirect_ostream(&mut sess, opt_stdio.stdout, &stdio, &opt.stdout_redirect)?;
        redirect_ostream(&mut sess, opt_stdio.stderr, &stdio, &opt.stderr_redirect)?;
    }

    sess.spawn()?.wait()
}

pub fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() == 1 {
        println!("{}", Options::help());
    } else {
        let reports = match run(&args[1..]) {
            Err(e) => {
                println!("{}", e.to_string());
                return;
            }
            Ok(x) => x,
        };
        for report in &reports {
            print_report(report);
        }
    }
}
