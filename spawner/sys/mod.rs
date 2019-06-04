use cfg_if::cfg_if;

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

pub trait AsInnerMut<T> {
    fn as_inner_mut(&mut self) -> &mut T;
}

pub trait AsInner<T> {
    fn as_inner(&self) -> &T;
}

pub trait FromInner<T> {
    fn from_inner(inner: T) -> Self;
}
