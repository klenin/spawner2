use crate::sys::error::last_os_error;

use backtrace::Backtrace;

use std::fmt;
use std::io;

#[derive(Debug)]
enum ErrorKind {
    Io(io::Error),
    Str(String),
    StaticStr(&'static str),
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    backtrace: Backtrace,
}

impl Error {
    fn new(k: ErrorKind) -> Self {
        Self {
            kind: k,
            backtrace: Backtrace::new(),
        }
    }

    pub fn last_os_error() -> Self {
        Self::from(last_os_error())
    }

    pub fn call_stack(&self) -> String {
        format!("{:?}", self.backtrace)
    }

    pub fn into_io_error(self) -> io::Error {
        match self.kind {
            ErrorKind::Io(e) => e,
            ErrorKind::Str(s) => io::Error::new(io::ErrorKind::Other, s),
            ErrorKind::StaticStr(s) => io::Error::new(io::ErrorKind::Other, s),
        }
    }
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            ErrorKind::Io(e) => write!(f, "{}", e),
            ErrorKind::Str(s) => write!(f, "{}", s),
            ErrorKind::StaticStr(s) => write!(f, "{}", s),
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::new(ErrorKind::Io(err))
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::new(ErrorKind::Str(s))
    }
}

impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Error::new(ErrorKind::StaticStr(s))
    }
}
