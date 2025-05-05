// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("relay: internal error: {0}")]
    InternalError(String),

    #[error("relay: nostr relay builder error: {0}")]
    NostrRelayBuilderError(#[from] nostr_relay_builder::Error),
}

impl RelayError {
    // FIXME
    // pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
    //     RelayError::InvalidArgument(msg.into())
    // }
    // pub fn missing_parameter<S: Into<String>>(msg: S) -> Self {
    //     RelayError::MissingParameter(msg.into())
    // }
    // pub fn operation_not_allowed<S: Into<String>>(msg: S) -> Self {
    //     RelayError::OperationNotAllowed(msg.into())
    // }
    pub fn internal_error<S: Into<String>>(msg: S) -> Self {
        RelayError::InternalError(msg.into())
    }
}

pub type RelayResult<T> = Result<T, RelayError>;
