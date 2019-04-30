use cfg_if::cfg_if;

mod limit_checker;

cfg_if! {
    if #[cfg(windows)] {
        mod windows;
        pub use self::windows::*;
    } else if #[cfg(unix)] {
        mod unix;
        pub use self::unix::*;
    } else {
        compile_error!("spawner doesn't compile for this platform yet");
    }
}

pub trait IntoInner<T> {
    fn into_inner(self) -> T;
}
