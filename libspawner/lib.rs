extern crate cfg_if;

#[cfg(windows)]
extern crate winapi;

pub mod command;
pub mod runner;
pub mod spawner;

mod sys;

pub use self::spawner::*;
