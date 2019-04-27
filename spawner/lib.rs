extern crate backtrace;
extern crate cfg_if;

#[cfg(windows)]
extern crate winapi;

pub mod iograph;
pub mod pipe;
pub mod process;
pub mod runner;
pub mod rwhub;
pub mod task;

pub use error::*;
pub type Result<T> = std::result::Result<T, self::Error>;

mod error;
mod sys;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
