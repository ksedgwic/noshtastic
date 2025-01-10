// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use async_trait::async_trait;
use prost::Message;
use sha2::{Digest, Sha256};
use std::convert::From;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

pub mod error;
pub use error::*;

mod proto {
    include!("../protos/noshtastic_link.rs");
}
mod fragcache;
mod serial;

pub(crate) use fragcache::FragmentCache;
pub(crate) use proto::{link_frame::Payload, LinkFrag, LinkFrame, LinkMsg};

pub type LinkRef = Arc<tokio::sync::Mutex<dyn MeshtasticLink>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMessage {
    pub data: Vec<u8>,
}

impl LinkMessage {
    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl From<Vec<u8>> for LinkMessage {
    fn from(buffer: Vec<u8>) -> Self {
        LinkMessage { data: buffer }
    }
}

impl From<meshtastic::protobufs::FromRadio> for LinkMessage {
    fn from(from_radio: meshtastic::protobufs::FromRadio) -> Self {
        LinkMessage {
            data: from_radio.encode_to_vec(),
        }
    }
}

impl From<LinkMsg> for LinkMessage {
    fn from(msg: LinkMsg) -> Self {
        LinkMessage { data: msg.data }
    }
}

#[async_trait]
pub trait MeshtasticLink: Send + Sync + Debug {
    // no longer needed, but lazy deleting in case we need again ...
}

pub async fn create_link(
    maybe_serial: &Option<String>,
    stop_signal: Arc<Notify>,
) -> LinkResult<(
    LinkRef,
    mpsc::Sender<LinkMessage>,
    mpsc::Receiver<LinkMessage>,
)> {
    // In the future we may have radio interfaces other than serial ...
    serial::SerialLink::create_serial_link(maybe_serial, stop_signal).await
}

// The PayloadId is a 64 bit message identifier, because other ids
// are too short or too long ...
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadId(u64);

impl fmt::Display for PayloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for PayloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PayloadId({:016x})", self.0)
    }
}

impl From<&[u8]> for PayloadId {
    fn from(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        let bytes = &hash[..8]; // Take the first 8 bytes
        PayloadId(u64::from_be_bytes(
            bytes.try_into().expect("Slice has incorrect length"),
        ))
    }
}
