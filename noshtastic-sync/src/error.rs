// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

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

    #[error("sync: nostrdb error: {0}")]
    NostrdbError(#[from] nostrdb::Error),

    #[error("sync: negentropy error: {0}")]
    NegentropyError(#[from] negentropy::Error),
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
