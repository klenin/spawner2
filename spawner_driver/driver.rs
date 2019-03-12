use crate::opts::Options;
use crate::report::Report;
use crate::session::SessionBuilderEx;

use json::JsonValue;

use spawner::{Error, Result};

use spawner_opts::CmdLineOptions;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};

pub struct Driver(Vec<Options>);

impl Driver {
    pub fn from_argv<T, U>(argv: T) -> Result<Self>
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let argv: Vec<String> = argv.into_iter().map(|x| x.as_ref().to_string()).collect();
        let mut default_opts = Options::from_env()?;
        let mut pos = 0;
        let mut cmds: Vec<Options> = Vec::new();
        let mut controller_exists = false;

        while pos < argv.len() {
            let mut opts = default_opts.clone();
            let num_opts = match opts.parse(&argv[pos..]) {
                Ok(n) => n,
                Err(s) => return Err(Error::from(s)),
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
                if opts.controller {
                    return Err(Error::from("Controller must have an argv"));
                }
                default_opts = opts;
            } else if opts.controller && controller_exists {
                return Err(Error::from("There can be at most one controller"));
            } else {
                controller_exists = controller_exists || opts.controller;
                default_opts.separator = opts.separator.clone();
                cmds.push(opts);
            }
        }

        Ok(Driver(cmds))
    }

    pub fn run(self) -> Result<Vec<Report>> {
        let runner_reports = SessionBuilderEx::from_cmds(&self.0)?.spawn()?.wait();
        let reports: Vec<Report> = runner_reports
            .into_iter()
            .zip(self.0.iter())
            .map(|(result, opts)| Report::new(opts, result))
            .collect();

        if reports.is_empty() {
            Options::print_help();
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
