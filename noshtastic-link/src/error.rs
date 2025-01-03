// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum LinkError {
    #[error("link: invalid argument: {0}")]
    InvalidArgument(String),

    #[error("link: missing required parameter: {0}")]
    MissingParameter(String),

    #[error("link: internal error: {0}")]
    InternalError(String),

    #[error("link: serial error: {0}")]
    SerialError(#[from] tokio_serial::Error),

    #[error("link: meshtastic error: {0}")]
    MeshtasticError(#[from] meshtastic::errors::Error),
}

impl From<mpsc::error::SendError<crate::LinkMessage>> for LinkError {
    fn from(err: mpsc::error::SendError<crate::LinkMessage>) -> Self {
        LinkError::InternalError(format!("failed to send message via channel: {:?}", err))
    }
}

impl LinkError {
    pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
        LinkError::InvalidArgument(msg.into())
    }
    pub fn missing_parameter<S: Into<String>>(msg: S) -> Self {
        LinkError::MissingParameter(msg.into())
    }
    pub fn internal_error<S: Into<String>>(msg: S) -> Self {
        LinkError::InternalError(msg.into())
    }
}

pub type LinkResult<T> = Result<T, LinkError>;
