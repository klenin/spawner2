use crate::Error;

use nix::errno::Errno;

use std::fmt;

#[derive(Debug)]
pub struct SysError(Errno);

impl SysError {
    pub fn last() -> Self {
        Self(Errno::last())
    }
}

impl std::error::Error for SysError {}

impl fmt::Display for SysError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0.desc())
    }
}

impl From<nix::Error> for Error {
    fn from(e: nix::Error) -> Error {
        match e.as_errno() {
            Some(errno) => Error::from(SysError(errno)),
            None => Error::from(e.to_string()),
        }
    }
}
