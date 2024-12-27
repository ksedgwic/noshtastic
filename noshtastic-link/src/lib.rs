use async_trait::async_trait;
use prost::Message;
use std::fmt::Debug;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::mpsc;

use proto::LinkFrame;

pub mod error;
pub use error::*;

pub mod serial; // serial connection to radio

pub mod proto {
    include!("../protos/noshtastic_link.rs");
}

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

pub struct LinkLayer {
    version: u32, // Define the current link version
}

impl LinkLayer {
    pub fn new(version: u32) -> Self {
        Self { version }
    }

    pub fn frame_data(&self, data: &[u8]) -> Vec<u8> {
        // Create a new LinkFrame
        let frame = LinkFrame {
            version: self.version,
            data: data.to_vec(),
        };

        // Serialize the frame to bytes
        let mut buffer = Vec::new();
        frame.encode(&mut buffer).unwrap();
        buffer
    }

    pub fn parse_frame(&self, frame_data: &[u8]) -> Option<Vec<u8>> {
        // Deserialize the bytes into a LinkFrame
        let frame = LinkFrame::decode(&mut Cursor::new(frame_data)).ok()?;
        if frame.version != self.version {
            return None; // Version mismatch
        }
        Some(frame.data)
    }
}
