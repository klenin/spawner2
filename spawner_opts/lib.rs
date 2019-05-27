//! This library contains `CmdLineOptions` and `OptionValueParser` traits along with
//! some definitions used by `spawner_opts_macro` crate.
//!
//! # Examples
//! ```
//! use spawner_opts::*;
//!
//! #[derive(CmdLineOptions)]
//! #[optcont(
//!     delimeters = "=",
//!     usage = "tool [options]",
//! )]
//! struct Opts {
//!     #[flag(name = "-f", desc = "a flag")]
//!     flag: bool,
//!     
//!     #[opt(
//!         names("-v", "--v"),
//!         desc = "an option",
//!         value_desc = "<float>",
//!         env = "ENV_VAR",
//!         parser = "FloatingLiteralParser"
//!     )]
//!     opt: f64,
//! }
//!
//! struct FloatingLiteralParser;
//!
//! impl OptionValueParser<f64> for FloatingLiteralParser {
//!     fn parse(opt: &mut f64, v: &str) -> Result<(), String> {
//!         match v.parse::<f64>() {
//!             Ok(x) => {
//!                 *opt = x;
//!                 Ok(())
//!             }
//!             Err(_) => Err(format!("Invalid value '{}'", v)),
//!         }
//!     }
//! }
//! ```

extern crate spawner_opts_derive;

pub mod parser;

pub use spawner_opts_derive::*;
use std::fmt;

pub struct OptionHelp {
    pub names: Vec<String>,
    pub desc: Option<String>,
    pub value_desc: Option<String>,
    pub env: Option<String>,
}

pub struct Help {
    pub overview: Option<String>,
    pub usage: Option<String>,
    pub delimeters: Option<String>,
    pub options: Vec<OptionHelp>,
}

pub trait CmdLineOptions: Sized {
    fn help() -> Help;
    fn parse_argv<T, U>(&mut self, argv: T) -> Result<usize, String>
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>;

    fn parse_env(&mut self) -> Result<(), String>;
}

pub trait OptionValueParser<T> {
    fn parse(opt: &mut T, val: &str) -> Result<(), String>;
}

impl fmt::Display for Help {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(ref overview) = self.overview {
            write!(f, "Overview: {}\n\n", overview)?;
        }
        if let Some(ref usage) = self.usage {
            write!(f, "Usage: {}\n\n", usage)?;
        }
        if self.options.is_empty() {
            return Ok(());
        }

        let delim = match self.delimeters {
            Some(ref d) => d.chars().next().unwrap_or(' '),
            None => ' ',
        };
        f.write_str("Options:\n")?;
        for opt in self.options.iter() {
            write_opt(f, opt, delim)?;
        }

        if self.options.iter().any(|opt| opt.env.is_some()) {
            f.write_str("\nEnvironment variables and corresponding options:\n")?;
            for opt in self.options.iter() {
                write_env_desc(f, opt)?;
            }
        }
        Ok(())
    }
}

fn write_env_desc(f: &mut fmt::Formatter, opt: &OptionHelp) -> fmt::Result {
    if let Some(ref env) = opt.env {
        let indent = "  ";
        let spaces = 30 - (env.len() + indent.len());
        write!(f, "{}{}{:3$}", indent, env, " ", spaces)?;
        for (idx, name) in opt.names.iter().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            f.write_str(name)?;
        }
        f.write_str("\n")?;
    }
    Ok(())
}

fn write_names(f: &mut fmt::Formatter, opt: &OptionHelp, delim: char) -> Result<usize, fmt::Error> {
    let mut names_len = 0;
    for (no, name) in opt.names.iter().enumerate() {
        if no > 0 {
            f.write_str(", ")?;
            names_len += 2;
        }
        f.write_str(name)?;
        names_len += name.len();
        if let Some(ref vd) = opt.value_desc {
            write!(f, "{}{}", delim, vd)?;
            names_len += 1 + vd.len();
        }
    }
    Ok(names_len)
}

fn write_opt(f: &mut fmt::Formatter, opt: &OptionHelp, delim: char) -> fmt::Result {
    let desc_offset = 30;
    let opt_offset = 2;
    let empty = &String::new();

    write!(f, "{:1$}", " ", opt_offset)?;
    let written = opt_offset + write_names(f, opt, delim)?;

    for (no, line) in opt
        .desc
        .as_ref()
        .unwrap_or(empty)
        .split("\n")
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        if no == 0 && written < desc_offset {
            write!(f, "{:1$}", " ", desc_offset - written)?;
        } else {
            write!(f, "\n{:1$}", " ", desc_offset)?;
        }
        f.write_str(line)?;
    }
    f.write_str("\n")?;
    Ok(())
}
