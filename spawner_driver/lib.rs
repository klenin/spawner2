extern crate json;
extern crate spawner;
extern crate spawner_opts;

mod cmd;
mod driver;
mod misc;
mod protocol;
mod report;
mod value_parser;

#[cfg(test)]
mod tests;

pub use crate::report::*;

use crate::cmd::Command;
use crate::driver::Driver;

use json::JsonValue;

use spawner::{Error, Result};

use spawner_opts::CmdLineOptions;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

struct Commands(Vec<Command>);

pub fn run<T, U>(argv: T) -> Result<Vec<Report>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    Commands::from_argv(argv).and_then(|cmds| cmds.run())
}

impl Commands {
    pub fn from_argv<T, U>(argv: T) -> Result<Self>
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let argv: Vec<String> = argv.into_iter().map(|x| x.as_ref().to_string()).collect();
        let mut default_cmd = Command::from_env()?;
        let mut pos = 0;
        let mut cmds: Vec<Command> = Vec::new();
        let mut controller_exists = false;

        while pos < argv.len() {
            let mut cmd = default_cmd.clone();
            pos += cmd.parse(&argv[pos..]).map_err(Error::from)?;

            let mut sep_pos = argv.len();
            if let Some(sep) = &cmd.separator {
                let full_sep = format!("--{}", sep);
                if let Some(i) = argv[pos..].iter().position(|x| x == &full_sep) {
                    sep_pos = pos + i;
                }
            }
            cmd.argv.extend_from_slice(&argv[pos..sep_pos]);
            pos = sep_pos + 1;

            if cmd.argv.is_empty() {
                if cmd.controller {
                    return Err(Error::from("Controller must have an argv"));
                }
                default_cmd = cmd;
            } else if cmd.controller && controller_exists {
                return Err(Error::from("There can be at most one controller"));
            } else {
                controller_exists = controller_exists || cmd.controller;
                default_cmd.separator = cmd.separator.clone();
                cmds.push(cmd);
            }
        }

        Ok(Commands(cmds))
    }

    pub fn run(self) -> Result<Vec<Report>> {
        let runner_reports = Driver::from_cmds(&self.0)?.spawn()?.wait();
        let reports: Vec<Report> = runner_reports
            .into_iter()
            .zip(self.0.iter())
            .map(|(result, opts)| Report::new(opts, result))
            .collect();

        if reports.is_empty() {
            Command::print_help();
        } else {
            self.print_reports(&reports)?;
        }
        Ok(reports)
    }

    fn print_reports(&self, reports: &Vec<Report>) -> io::Result<()> {
        let mut output_files: HashMap<&String, Vec<&Report>> = HashMap::new();
        for (i, cmd) in self.0.iter().enumerate() {
            if !cmd.hide_report && reports.len() == 1 {
                println!("{}", reports[i]);
            }
            if let Some(filename) = &cmd.output_file {
                output_files
                    .entry(filename)
                    .or_insert(Vec::new())
                    .push(&reports[i]);
            }
        }

        for (filename, file_reports) in output_files.into_iter() {
            let _ = fs::remove_file(filename);
            let mut file = fs::File::create(filename)?;

            if file_reports.len() == 1 && !file_reports[0].kind.is_json() {
                write!(&mut file, "{}", file_reports[0])?;
            } else if file_reports.iter().all(|r| r.kind.is_json()) {
                let json_reports =
                    JsonValue::Array(file_reports.into_iter().map(|r| r.to_json()).collect());
                json_reports.write_pretty(&mut file, 4)?;
            }
        }

        Ok(())
    }
}
