// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use async_trait::async_trait;
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
    pub msgid: MsgId,
    pub data: Vec<u8>,
}

impl LinkMessage {
    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }
}

// impl From<Vec<u8>> for LinkMessage {
//     fn from(buffer: Vec<u8>) -> Self {
//         LinkMessage { data: buffer }
//     }
// }

// impl From<meshtastic::protobufs::FromRadio> for LinkMessage {
//     fn from(from_radio: meshtastic::protobufs::FromRadio) -> Self {
//         LinkMessage {
//             data: from_radio.encode_to_vec(),
//         }
//     }
// }

impl From<LinkMsg> for LinkMessage {
    fn from(msg: LinkMsg) -> Self {
        LinkMessage {
            msgid: MsgId(msg.msgid),
            data: msg.data,
        }
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

/// The nostr msgid for notes or a content hash for other messages
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MsgId(pub u64);

impl MsgId {
    /// Constructs a `MsgId` from the first 8 bytes of a 32-byte nostr msgid
    pub fn from_nostr_msgid(data: &[u8; 32]) -> Self {
        let first_8_bytes = &data[..8]; // Take the first 8 bytes
        MsgId(u64::from_be_bytes(
            first_8_bytes
                .try_into()
                .expect("Slice has incorrect length"),
        ))
    }
}

impl fmt::Display for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MsgId({:016x})", self.0)
    }
}

impl From<&[u8]> for MsgId {
    fn from(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        let bytes = &hash[..8]; // Take the first 8 bytes
        MsgId(u64::from_be_bytes(
            bytes.try_into().expect("Slice has incorrect length"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex;

    #[test]
    fn test_from_nostr_msgid() {
        // Example 32-byte Nostr message ID
        let nostr_msgid = "3127fa5371d17771514829bd4067003dadac0f847d3055c5e4a6fd560c3467f9";
        let nostr_bytes = hex::decode(nostr_msgid).expect("Invalid hex string");

        // Generate MsgId from the first 8 bytes
        let msg_id = MsgId::from_nostr_msgid(nostr_bytes.as_slice().try_into().unwrap());
        let msg_id_str = format!("{}", msg_id);

        assert_eq!(
            &nostr_msgid[..16],
            &msg_id_str[..16],
            "First 16 characters do not match"
        );

        let asint: u64 = msg_id.0;
        let msgid2 = MsgId(asint);
        let msg_id2_str = format!("{}", msgid2);
        assert_eq!(msg_id_str, msg_id2_str);
    }
}
