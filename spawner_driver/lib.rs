extern crate chardet;
extern crate encoding;
extern crate json;
extern crate spawner;
extern crate spawner_opts;

#[cfg(windows)]
extern crate winapi;

#[cfg(unix)]
extern crate libc;

mod cmd;
mod driver;
mod misc;
mod protocol_entities;
mod protocol_handlers;
mod report;
mod sys;
mod value_parser;

#[cfg(test)]
mod tests;

pub use crate::report::*;

use crate::driver::Driver;

use spawner::Result;

pub fn run<T, U>(argv: T) -> Result<Vec<Report>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    Driver::from_argv(argv).and_then(|d| d.run())
}
