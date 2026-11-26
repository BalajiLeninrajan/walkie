use std::{fmt, io, path::PathBuf};

/// Everything that can stop a dictation.
#[derive(Debug)]
pub enum Error {
    /// A line in the config file couldn't be parsed or had a bad value.
    Config { path: PathBuf, line: usize, msg: String },
    /// A file or socket operation failed.
    Io { what: String, source: io::Error },
    /// PipeWire refused to connect or record.
    Audio(String),
    /// transcribe.cpp couldn't load the model or run it.
    Engine(String),
    /// The Wayland connection or the virtual keyboard failed.
    Wayland(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { path, line, msg } => write!(f, "{}:{line}: {msg}", path.display()),
            Self::Io { what, source } => write!(f, "{what}: {source}"),
            Self::Audio(msg) => write!(f, "audio: {msg}"),
            Self::Engine(msg) => write!(f, "transcriber: {msg}"),
            Self::Wayland(msg) => write!(f, "wayland: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Adds a description to an `io::Result`, like `anyhow::Context` but typed.
pub trait IoContext<T> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn ctx(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|source| Error::Io { what: what(), source })
    }
}
