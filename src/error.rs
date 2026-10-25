//! 入力・競合・権限・保存障害を区別し、HTTPへの対応を一箇所で定める。
use std::io;
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Internal(#[source] io::Error),
}
impl Failure {
    pub fn status(&self) -> u16 {
        match self {
            Self::Input(_) => 400,
            Self::Conflict(_) => 409,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Timeout(_) => 504,
            Self::Internal(_) => 500,
        }
    }
}
impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => {
                Self::Input(error.to_string())
            }
            io::ErrorKind::AlreadyExists | io::ErrorKind::WouldBlock => {
                Self::Conflict(error.to_string())
            }
            io::ErrorKind::PermissionDenied => Self::Forbidden(error.to_string()),
            io::ErrorKind::NotFound => Self::NotFound(error.to_string()),
            io::ErrorKind::TimedOut => Self::Timeout(error.to_string()),
            _ => Self::Internal(error),
        }
    }
}
pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, Failure::Input(message.into()))
}
