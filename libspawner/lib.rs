extern crate backtrace;
extern crate cfg_if;
extern crate spawner_opts;

#[cfg(windows)]
extern crate winapi;

pub mod command;
pub mod driver;
pub mod runner;
pub mod session;

pub mod process {
    pub use sys::process::*;
}

pub mod pipe {
    pub use sys::pipe::*;
}

pub use error::*;
pub type Result<T> = std::result::Result<T, self::Error>;

mod error;
mod io;
mod runner_private;
mod sys;
