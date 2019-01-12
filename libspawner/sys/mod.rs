use cfg_if::cfg_if;

mod process_common;

pub use self::process_common::*;

cfg_if! {
    if #[cfg(windows)] {
        extern crate winapi;
        mod windows;
        pub use self::windows::*;
    } else {
        compile_error!("spawner doesn't compile for this platform yet");
    }
}
