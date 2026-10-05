use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// The application path does not exist
    NotFound(String),
    /// Platform API call failed
    Platform(String),
    /// The application has already terminated
    Terminated,
    /// Operation not supported on this platform
    Unsupported,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(path) => write!(f, "application not found: {path}"),
            Error::Platform(msg) => write!(f, "platform error: {msg}"),
            Error::Terminated => write!(f, "application has terminated"),
            Error::Unsupported => write!(f, "operation not supported on this platform"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for io::Error {
    fn from(err: Error) -> io::Error {
        match err {
            Error::NotFound(path) => io::Error::new(io::ErrorKind::NotFound, path),
            Error::Platform(msg) => io::Error::new(io::ErrorKind::Other, msg),
            Error::Terminated => io::Error::new(io::ErrorKind::BrokenPipe, "application has terminated"),
            Error::Unsupported => io::Error::new(io::ErrorKind::Unsupported, "operation not supported on this platform"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
