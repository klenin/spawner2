extern crate json;
extern crate spawner;
extern crate spawner_opts;

mod driver;
mod misc;
mod opts;
mod protocol;
mod report;
mod session;
mod value_parser;

#[cfg(test)]
mod tests;

pub use crate::report::*;

pub fn run<T, U>(argv: T) -> spawner::Result<Vec<Report>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    driver::Driver::from_argv(argv).and_then(|driver| driver.run())
}
