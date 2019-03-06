extern crate backtrace;
extern crate cfg_if;

#[cfg(windows)]
extern crate winapi;

pub mod command;
pub mod pipe;
pub mod runner;
pub mod session;
pub mod stdio;

pub use error::*;
pub type Result<T> = std::result::Result<T, self::Error>;

mod error;
mod runner_private;
mod sys;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
