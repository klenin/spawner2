extern crate backtrace;
extern crate cfg_if;

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(windows)] {
        extern crate winapi;

        pub mod windows {
            pub use sys::process_ext as process;
            pub use sys::pipe_ext as pipe;
        }
    } else if #[cfg(unix)] {
        extern crate nix;
        extern crate rand;
        extern crate cgroups_fs;
        extern crate procfs;

        pub mod unix {
            pub use sys::process_ext as process;
        }
    }
}

pub mod io;
pub mod pipe;
pub mod process;
pub mod rwhub;

mod error;
mod limit_checker;
mod runner;
mod spawner;
mod sys;

pub use error::*;
pub use spawner::*;

pub type Result<T> = std::result::Result<T, self::Error>;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
