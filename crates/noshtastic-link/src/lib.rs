// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use meshtastic::utils;
use sha2::{Digest, Sha256};
use std::{convert::From, fmt, fmt::Debug, sync::Arc};
use tokio::sync::{mpsc, Mutex, Notify};

pub mod error;
pub use error::*;

mod proto {
    include!("../protos/noshtastic_link.rs");
}
mod fragcache;
mod outgoing;

// Android uses Bluetooth Low-Energy (BLE) to connect to the radio
// #[cfg(target_os = "android")]
mod ble_driver;

// CLI (unix) uses wired USB serial to connect to the radio
// #[cfg(not(target_os = "android"))]
mod usbserial_driver;

mod link;

pub use link::{Link, LinkRef};

pub(crate) use fragcache::FragmentCache;
pub(crate) use proto::{link_frame::Payload, LinkFrag, LinkFrame, LinkMsg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOptions {
    pub priority: Priority,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Drop,  // drop this msg if a match is already queued
    Queue, // ignore matches, just queue
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Action::Drop => "Drop",
            Action::Queue => "Queue",
        };
        write!(f, "{}", description)
    }
}

#[derive(Debug, Default)]
pub struct LinkOptionsBuilder {
    priority: Option<Priority>,
    action: Option<Action>,
}

impl LinkOptionsBuilder {
    pub fn new() -> Self {
        Self {
            priority: None,
            action: None,
        }
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.action = Some(action);
        self
    }

    pub fn build(self) -> LinkOptions {
        LinkOptions {
            priority: self.priority.unwrap_or(Priority::Normal), // Default priority
            action: self.action.unwrap_or(Action::Queue),        // Default action
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPayload {
    pub msgid: MsgId,
    pub options: LinkOptions,
    pub data: Vec<u8>,
}

impl LinkPayload {
    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkMessage {
    // link is ready, sent upstream to client
    Ready,
    // link data payload, used in both directions
    Payload(LinkPayload),
}

impl From<LinkMsg> for LinkMessage {
    fn from(msg: LinkMsg) -> Self {
        LinkMessage::Payload(LinkPayload {
            msgid: MsgId::new(msg.msgid, None),
            options: LinkOptionsBuilder::new().build(),
            data: msg.data,
        })
    }
}

pub async fn scan_for_radios() -> LinkResult<Vec<String>> {
    if cfg!(target_os = "android") {
        ble_driver::scan_for_ble_radios().await
    } else {
        Err(LinkError::invalid_argument(
            "scan only implemented for android",
        ))
    }
}

pub async fn create_link(
    maybe_hint: &Option<String>,
    stop_signal: Arc<Notify>,
) -> LinkResult<(
    LinkRef,
    mpsc::Sender<LinkMessage>,
    mpsc::Receiver<LinkMessage>,
)> {
    debug!("create_link starting");

    // create a stream to the radio
    let (mut mesh_in_rx, connected_stream_api) = if cfg!(target_os = "android") {
        ble_driver::create_ble_stream(maybe_hint).await
    } else {
        usbserial_driver::create_usbserial_stream(maybe_hint).await
    }?;

    debug!("stream_api configure starting");
    let config_id = utils::generate_rand_id();
    let configured_stream_api = connected_stream_api.configure(config_id).await?;
    wait_for_config_complete(&mut mesh_in_rx, config_id).await?;
    debug!("stream_api configure finished");

    // create some channels
    // +-----------+               +------------+              +------------+
    // |           |   client_in   |            |              |            |
    // |  Sync  tx | ------------> | rx  Link   |              |   Radio    |
    // |           |   client_out  |            |    mesh_in   |            |
    // |        rx | <------------ | tx      rx | <----------- | tx         |
    // +-----------+               +------------+              +------------+

    let (client_in_tx, client_in_rx) = mpsc::channel::<LinkMessage>(100);
    let (client_out_tx, client_out_rx) = mpsc::channel::<LinkMessage>(100);

    // create the link
    let linkref = Arc::new(Mutex::new(Link::new(
        configured_stream_api,
        client_out_tx,
        stop_signal.clone(),
    )));

    // start assoiated tasks
    Link::start(&linkref, mesh_in_rx, client_in_rx, stop_signal.clone()).await?;

    debug!("create_link finished");
    Ok((linkref, client_in_tx, client_out_rx))
}

use meshtastic::protobufs;
use meshtastic::protobufs::from_radio::PayloadVariant;

pub async fn wait_for_config_complete(
    decoded_listener: &mut mpsc::UnboundedReceiver<protobufs::FromRadio>,
    config_id: u32,
) -> LinkResult<()> {
    while let Some(packet) = decoded_listener.recv().await {
        // Check the payload_variant field
        if let Some(payload) = packet.payload_variant {
            match payload {
                PayloadVariant::ConfigCompleteId(id) => {
                    if id == config_id {
                        log::debug!("Received ConfigComplete for ID {id}, config is finished");
                        return Ok(());
                    } else {
                        log::info!("Got ConfigCompleteId for {id}, but expecting {config_id}");
                    }
                }
                PayloadVariant::MyInfo(myinfo) => {
                    log::info!("Saw MyNodeInfo => device ID: {}", myinfo.my_node_num,);
                }
                PayloadVariant::NodeInfo(nodeinfo) => {
                    log::info!(
                        "Saw NodeInfo => node num: {}, user: {:?}",
                        nodeinfo.num,
                        nodeinfo.user
                    );
                }
                PayloadVariant::Config(cfg) => {
                    log::info!("saw Config: {:?}", cfg);
                }
                PayloadVariant::Channel(ch) => {
                    log::info!(
                        "Saw Channel => index: {}, settings: {:?}",
                        ch.index,
                        ch.settings
                    );
                }
                PayloadVariant::ModuleConfig(mod_cfg) => {
                    log::info!("Saw ModuleConfig => partial config: {:?}", mod_cfg);
                }
                other => {
                    log::debug!("Saw: {:?}", other);
                }
            }
        } else {
            log::warn!("No payload_variant in this FromRadio message");
        }
    }

    // If we get here, the channel closed with no ConfigCompleteId found
    Err(LinkError::internal_error(
        "Channel closed before ConfigCompleteId",
    ))
}

/// The nostr msgid for notes or a content hash for other messages
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MsgId {
    pub base: u64,
    pub frag: Option<u32>,
}

impl MsgId {
    pub fn new(base: u64, frag: Option<u32>) -> Self {
        MsgId { base, frag }
    }

    /// Constructs a `MsgId` from the first 8 bytes of a 32-byte nostr msgid
    pub fn from_nostr_msgid(data: &[u8; 32]) -> Self {
        let first_8_bytes = &data[..8]; // Take the first 8 bytes
        MsgId {
            base: u64::from_be_bytes(first_8_bytes.try_into().unwrap()),
            frag: None,
        }
    }
}

// hashes the data and makes an id from the first 8 bytes
impl From<&[u8]> for MsgId {
    fn from(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        let bytes = &hash[..8]; // Take the first 8 bytes
        MsgId {
            base: u64::from_be_bytes(bytes.try_into().expect("Slice has incorrect length")),
            frag: None,
        }
    }
}

impl fmt::Display for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.base)?;
        if let Some(frag) = self.frag {
            write!(f, ":{}", frag)?;
        }
        Ok(())
    }
}

impl fmt::Debug for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MsgId({:016x}", self.base)?;
        if let Some(frag) = self.frag {
            write!(f, ":{}", frag)?;
        }
        write!(f, ")")
    }
}

// Encodes a binary buffer to avoid the 0x94C3 sequence and escapes 0xEE (See noshtastic#15)
pub(crate) fn escape94c3(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 1 < data.len() && data[i] == 0x94 && data[i + 1] == 0xC3 {
            out.push(0xEE);
            out.push(0x01);
            i += 2;
        } else if data[i] == 0xEE {
            out.push(0xEE);
            out.push(0x00);
            i += 1;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

// Decodes a buffer that was previously encoded with `encode` (See noshtastic#15)
pub(crate) fn unescape94c3(encoded: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] == 0xEE {
            if i + 1 >= encoded.len() {
                return Err("truncated escape sequence".into());
            }
            match encoded[i + 1] {
                0x00 => {
                    out.push(0xEE);
                    i += 2;
                }
                0x01 => {
                    out.push(0x94);
                    out.push(0xC3);
                    i += 2;
                }
                _ => return Err(format!("invalid escape code {:02x}", encoded[i + 1])),
            }
        } else {
            out.push(encoded[i]);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let asint: u64 = msg_id.base;
        let msgid2 = MsgId::new(asint, None);
        let msg_id2_str = format!("{}", msgid2);
        assert_eq!(msg_id_str, msg_id2_str);
    }

    #[test]
    fn test_escape_and_unescape_roundtrip() {
        let input = vec![0x10, 0x94, 0xC3, 0xEE, 0x20];
        let encoded = escape94c3(&input);
        let decoded = unescape94c3(&encoded).expect("decode failed");
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_escape_only_94c3() {
        let input = vec![0x94, 0xC3];
        let encoded = escape94c3(&input);
        assert_eq!(encoded, vec![0xEE, 0x01]);
    }

    #[test]
    fn test_escape_only_ee() {
        let input = vec![0xEE];
        let encoded = escape94c3(&input);
        assert_eq!(encoded, vec![0xEE, 0x00]);
    }

    #[test]
    fn test_link_payload_escape_roundtrip() {
        let payload = LinkPayload {
            msgid: MsgId::new(123, None),
            options: LinkOptionsBuilder::new().build(),
            data: vec![0x94, 0xC3, 0xEE, 0x01],
        };
        let escaped = payload.escape94c3();
        let unescaped = escaped.unescape94c3().expect("unescape failed");
        assert_eq!(unescaped, payload);
    }

    #[test]
    fn test_unescape_invalid_sequence() {
        let input = vec![0xEE, 0xFF];
        let result = unescape94c3(&input);
        assert!(result.is_err());
    }

    #[test]
    fn test_unescape_truncated_sequence() {
        let input = vec![0xEE];
        let result = unescape94c3(&input);
        assert!(result.is_err());
    }
}
