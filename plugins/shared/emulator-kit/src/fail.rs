//! A small error type for the package readers, with the `ensure!`, `anyhow!` and `.context()`
//! helpers they were written with (they came from the old app, which used `anyhow`).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn msg(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }

    /// The same error, with what was being done in front.
    pub fn context(self, context: impl fmt::Display) -> Self {
        Self(format!("{context}: {}", self.0))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<Error> for nexus_api::PluginError {
    fn from(e: Error) -> Self {
        nexus_api::PluginError::InvalidInput(e.0)
    }
}

/// `.context(...)` on a result or an option.
pub trait Context<T>: Sized {
    fn context(self, context: impl fmt::Display) -> Result<T>;
    fn with_context<D: fmt::Display>(self, context: impl FnOnce() -> D) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, context: impl fmt::Display) -> Result<T> {
        self.map_err(|e| Error(format!("{context}: {e}")))
    }
    fn with_context<D: fmt::Display>(self, context: impl FnOnce() -> D) -> Result<T> {
        self.map_err(|e| Error(format!("{}: {e}", context())))
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, context: impl fmt::Display) -> Result<T> {
        self.ok_or_else(|| Error::msg(context))
    }
    fn with_context<D: fmt::Display>(self, context: impl FnOnce() -> D) -> Result<T> {
        self.ok_or_else(|| Error::msg(context()))
    }
}

/// Returns an error unless the condition holds.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $msg:literal $(,)?) => {
        if !$cond {
            return Err($crate::fail::Error::msg(format!($msg)));
        }
    };
    ($cond:expr, $fmt:literal, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::fail::Error::msg(format!($fmt, $($arg)+)));
        }
    };
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return Err($crate::fail::Error::msg($err));
        }
    };
}

/// Returns an error from a message.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)+) => {
        return Err($crate::anyhow!($($arg)+))
    };
}

/// An error from a message.
#[macro_export]
macro_rules! anyhow {
    ($msg:literal $(,)?) => { $crate::fail::Error::msg(format!($msg)) };
    ($fmt:literal, $($arg:tt)+) => { $crate::fail::Error::msg(format!($fmt, $($arg)+)) };
}
