use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("sync: invalid argument: {0}")]
    InvalidArgument(String),

    #[error("sync: missing required parameter: {0}")]
    MissingParameter(String),

    #[error("sync: operation not allowed: {0}")]
    OperationNotAllowed(String), // An action violates some policy or constraint

    #[error("sync: internal error: {0}")]
    InternalError(String),

    #[error("sync: link error: {0}")]
    LinkError(#[from] noshtastic_link::LinkError),
}

impl SyncError {
    pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
        SyncError::InvalidArgument(msg.into())
    }
    pub fn missing_parameter<S: Into<String>>(msg: S) -> Self {
        SyncError::MissingParameter(msg.into())
    }
    pub fn operation_not_allowed<S: Into<String>>(msg: S) -> Self {
        SyncError::OperationNotAllowed(msg.into())
    }
    pub fn internal_error<S: Into<String>>(msg: S) -> Self {
        SyncError::InternalError(msg.into())
    }
}

pub type SyncResult<T> = Result<T, SyncError>;
