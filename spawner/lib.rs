extern crate backtrace;
extern crate cfg_if;
extern crate json;
extern crate spawner_opts;

#[cfg(windows)]
extern crate winapi;

pub mod command;
pub mod driver;
pub mod pipe;
pub mod runner;
pub mod session;

pub use error::*;
pub type Result<T> = std::result::Result<T, self::Error>;

mod error;
mod runner_private;
mod stdio;
mod sys;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
