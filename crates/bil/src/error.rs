// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The one error type every reader reports through (section 3.3, M45).

use std::{fmt, io};

/// What can go wrong opening or reading a Source.
#[derive(Debug)]
pub enum Error {
    /// No driver recognizes the bytes; the drivers tried, by name (M51).
    Unrecognized(Vec<String>),
    /// The file breaks its own structure: a length, count, offset or field that cannot be right.
    Corrupt(String),
    /// Valid data that this build does not read, or a read a limit refuses (section 10.3).
    Unsupported(String),
    /// A request outside what the Source holds: an Image, a level, a group, a field or a Window.
    NotFound(String),
    /// A failed read of the bytes.
    Io(io::Error),
    /// A failure of BIL's own, such as a panic caught at an interface: a bug in BIL, to be
    /// reported, never in the file or the call (section 3.3).
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unrecognized(tried) => {
                write!(
                    f,
                    "no driver recognizes the file; tried {}",
                    tried.join(", ")
                )
            }
            Error::Corrupt(m) => write!(f, "corrupt: {m}"),
            Error::Unsupported(m) => write!(f, "not supported: {m}"),
            Error::NotFound(m) => write!(f, "not found: {m}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Internal(m) => write!(f, "internal failure in BIL, to be reported: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
