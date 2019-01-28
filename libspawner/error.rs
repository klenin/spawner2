use backtrace::Backtrace;
use std::fmt;
use std::io;

#[derive(Debug)]
enum ErrorKind {
    Io(io::Error),
    Str(String),
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    call_stack: String,
}

impl Error {
    fn new(k: ErrorKind) -> Self {
        Self {
            kind: k,
            call_stack: format!("{:?}", Backtrace::new()),
        }
    }

    pub fn last_os_error() -> Self {
        Self::from(io::Error::last_os_error())
    }

    pub fn call_stack(&self) -> String {
        self.call_stack.clone()
    }

    pub fn into_io_error(self) -> io::Error {
        match self.kind {
            ErrorKind::Io(e) => e,
            ErrorKind::Str(s) => io::Error::new(io::ErrorKind::Other, s),
        }
    }
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.kind {
            ErrorKind::Io(e) => write!(f, "Error: {}\n", e.to_string()),
            ErrorKind::Str(s) => write!(f, "Error: {}\n", s.as_str()),
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
        Error::new(ErrorKind::Str(s.clone()))
    }
}

impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Error::new(ErrorKind::Str(s.to_string()))
    }
}
