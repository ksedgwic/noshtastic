use async_trait::async_trait;
use prost::Message;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod error;
pub use error::*;

pub mod serial; // serial connection to radio

pub mod proto {
    include!("../protos/noshtastic_link.rs");
}

pub use proto::{link_frame::Payload, LinkFrame, LinkMsg};

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
    /// Send a message frame
    async fn queue_message(&mut self, msg: LinkMessage) -> LinkResult<()>;
}

pub async fn create_link(
    maybe_serial: &Option<String>,
) -> LinkResult<(LinkRef, mpsc::Receiver<LinkMessage>)> {
    // In the future we may have radio interfaces other than serial ...
    serial::SerialLink::create_serial_link(maybe_serial).await
}
