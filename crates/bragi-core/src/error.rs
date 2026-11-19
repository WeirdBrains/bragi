//! Typed errors for both shells (the bridge turns them into Dart exceptions, never panics).

use weirdnet_agent::RemoteError;

/// What went wrong, in terms a shell can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// This device isn't linked to the user's Weirdnet yet.
    NotPaired,
    /// Linking failed, expired, was cancelled or the codes didn't match.
    Pairing,
    /// No inference node answered.
    Unavailable,
    /// Every inference node is full; try again shortly.
    Busy,
    /// The user stopped the answer.
    Cancelled,
    /// The node refused this device (not in the roster, or revoked).
    Refused,
    /// The request itself was wrong (unknown model, bad input).
    BadRequest,
    /// Reading or writing this device's files.
    Storage,
    /// No such conversation or request.
    NotFound,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CoreError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CoreError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl From<anyhow::Error> for CoreError {
    fn from(e: anyhow::Error) -> Self {
        Self::new(ErrorKind::Other, format!("{e:#}"))
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorKind::Storage, e.to_string())
    }
}

impl From<&RemoteError> for CoreError {
    fn from(e: &RemoteError) -> Self {
        let kind = match e {
            RemoteError::Busy => ErrorKind::Busy,
            RemoteError::Cancelled => ErrorKind::Cancelled,
            RemoteError::Refused { .. } => ErrorKind::Refused,
            RemoteError::BadRequest { .. } => ErrorKind::BadRequest,
            RemoteError::Unavailable { message } if message.starts_with("unknown model") => ErrorKind::BadRequest,
            RemoteError::Unavailable { .. } | RemoteError::Unreachable { .. } | RemoteError::Timeout | RemoteError::Closed => ErrorKind::Unavailable,
            _ => ErrorKind::Other,
        };
        Self::new(kind, e.to_string())
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
