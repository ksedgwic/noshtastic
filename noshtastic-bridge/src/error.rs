use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("bridge: invalid argument: {0}")]
    InvalidArgument(String),

    #[error("bridge: missing required parameter: {0}")]
    MissingParameter(String),

    #[error("bridge: operation not allowed: {0}")]
    OperationNotAllowed(String), // An action violates some policy or constraint

    #[error("bridge: internal error: {0}")]
    InternalError(String),

    #[error("bridge: enostr error: {0}")]
    EnostrError(#[from] enostr::Error),

    #[error("bridge: nostrdb error: {0}")]
    NostrdbError(#[from] nostrdb::Error),
}

impl BridgeError {
    pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
        BridgeError::InvalidArgument(msg.into())
    }
    pub fn missing_parameter<S: Into<String>>(msg: S) -> Self {
        BridgeError::MissingParameter(msg.into())
    }
    pub fn operation_not_allowed<S: Into<String>>(msg: S) -> Self {
        BridgeError::OperationNotAllowed(msg.into())
    }
    pub fn internal_error<S: Into<String>>(msg: S) -> Self {
        BridgeError::InternalError(msg.into())
    }
}

pub type BridgeResult<T> = Result<T, BridgeError>;
