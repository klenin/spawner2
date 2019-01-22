extern crate cfg_if;

#[cfg(windows)]
extern crate winapi;

pub mod command;
pub mod runner;

pub mod process {
    pub use sys::process::*;
}

pub mod pipe {
    pub use sys::pipe::*;
}

pub use self::spawner::*;

mod internals;
mod spawner;
mod sys;
