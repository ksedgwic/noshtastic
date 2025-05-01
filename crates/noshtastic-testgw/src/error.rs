// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestGWError {
    #[error("testgw: invalid argument: {0}")]
    InvalidArgument(String),

    #[error("testgw: missing required parameter: {0}")]
    MissingParameter(String),

    #[error("testgw: operation not allowed: {0}")]
    OperationNotAllowed(String), // An action violates some policy or constraint

    #[error("testgw: internal error: {0}")]
    InternalError(String),

    #[error("testgw: nostr_relay_pool error: {0}")]
    RelayPoolError(#[from] nostr_relay_pool::pool::Error),

    #[error("testgw: serde_json error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    #[error("testgw: nostrdb error: {0}")]
    NostrdbError(#[from] nostrdb::Error),
}

impl TestGWError {
    pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
        TestGWError::InvalidArgument(msg.into())
    }
    pub fn missing_parameter<S: Into<String>>(msg: S) -> Self {
        TestGWError::MissingParameter(msg.into())
    }
    pub fn operation_not_allowed<S: Into<String>>(msg: S) -> Self {
        TestGWError::OperationNotAllowed(msg.into())
    }
    pub fn internal_error<S: Into<String>>(msg: S) -> Self {
        TestGWError::InternalError(msg.into())
    }
}

pub type TestGWResult<T> = Result<T, TestGWError>;
