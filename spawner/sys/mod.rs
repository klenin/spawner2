use cfg_if::cfg_if;

mod limit_checker;

cfg_if! {
    if #[cfg(windows)] {
        extern crate winapi;
        mod windows;
        pub use self::windows::*;
    } else {
        compile_error!("spawner doesn't compile for this platform yet");
    }
}

pub trait IntoInner<T> {
    fn into_inner(self) -> T;
}
