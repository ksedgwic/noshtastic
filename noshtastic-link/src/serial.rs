// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use async_trait::async_trait;
use log::*;
use meshtastic::{
    api::{ConnectedStreamApi, StreamApi},
    packet::{PacketDestination, PacketRouter},
    protobufs::{from_radio, mesh_packet, FromRadio, MeshPacket, PortNum},
    types::NodeId,
    utils,
};
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, Mutex, Notify},
    task,
    time::{sleep, Duration},
};

use crate::{
    LinkError, LinkFrag, LinkFrame, LinkMessage, LinkMsg, LinkRef, LinkResult, MeshtasticLink,
    Payload,
};

const LINK_VERSION: u32 = 1;
const LINK_MAGIC: u32 = 0x48534F4E; // 'NOSH'
const LINK_FRAG_THRESH: usize = 200;

// Fragments share a common MsgId which is derived from a hash
// of the whole message.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MsgId(u64);

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

#[allow(dead_code)] // FIXME - remove this asap
#[derive(Debug)]
struct PartialMsg {
    inbound: bool,
    created: u64, // first seen
    lasttry: u64, // last retry
    nretries: u32,
    frags: Vec<Vec<u8>>, // missing fragments are zero-sized
}

#[derive(Debug)]
struct FragmentCache {
    partials: BTreeMap<MsgId, PartialMsg>,
}

impl FragmentCache {
    fn new() -> Self {
        FragmentCache {
            partials: BTreeMap::new(),
        }
    }

    fn add_fragment(&mut self, frag: &LinkFrag, inbound: bool) -> Option<LinkMsg> {
        // Retrieve or create a new PartialMsg
        let partial = self.partials.entry(MsgId(frag.msgid)).or_insert_with(|| {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            let frags = vec![Vec::new(); frag.numfrag as usize];
            PartialMsg {
                inbound,
                created: timestamp,
                lasttry: timestamp, // not a retry, but used to schedulefirst retry
                nretries: 0,
                frags,
            }
        });

        // Update the fragment
        if frag.fragndx < partial.frags.len() as u32 {
            partial.frags[frag.fragndx as usize] = frag.data.clone();
        } else {
            error!("invalid fragment index: {}", frag.fragndx);
            return None; // Invalid fragment index
        }

        if inbound {
            // Check if all fragments are present
            if partial.frags.iter().all(|f| !f.is_empty()) {
                // Assemble the complete message
                let mut complete_data = Vec::new();
                for fragment in &partial.frags {
                    complete_data.extend_from_slice(fragment);
                }

                // we don't purge the PartialMsg right away so it will be
                // available if other nodes need retries

                // Return the complete LinkMsg
                return Some(LinkMsg {
                    data: complete_data,
                });
            }
        }

        // Return None if the message is not yet complete
        None
    }
}

#[derive(Debug)]
pub struct SerialLink {
    stream_api: ConnectedStreamApi,
    client_out_tx: mpsc::Sender<LinkMessage>,
    _stop_signal: Arc<Notify>,
    my_node_num: u32,
    fragcache: FragmentCache,
}
pub type SerialLinkRef = Arc<tokio::sync::Mutex<SerialLink>>;

impl SerialLink {
    pub fn new(
        stream_api: ConnectedStreamApi,
        client_out_tx: mpsc::Sender<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) -> Self {
        SerialLink {
            stream_api,
            client_out_tx,
            _stop_signal: stop_signal,
            my_node_num: 0,
            fragcache: FragmentCache::new(),
        }
    }

    fn set_my_node_num(&mut self, my_node_num: u32) {
        info!("setting my_node_num: {}", my_node_num);
        self.my_node_num = my_node_num;
    }

    pub async fn create_serial_link(
        maybe_serial: &Option<String>,
    ) -> LinkResult<(
        LinkRef,
        mpsc::Sender<LinkMessage>,
        mpsc::Receiver<LinkMessage>,
    )> {
        let serial = match maybe_serial.clone() {
            Some(serial) => serial, // specified, just use
            None => {
                debug!("querying available serial ports ...");
                let available_ports = utils::stream::available_serial_ports()?;

                match available_ports.as_slice() {
                    [port] => port.clone(), // exactly one port available
                    [] => {
                        return Err(LinkError::missing_parameter(
                            "No available serial ports found. Use --serial to specify.".to_string(),
                        ));
                    }
                    _ => {
                        return Err(LinkError::missing_parameter(format!(
                            "Multiple available serial ports found: {:?}. Use --serial to specify.",
                            available_ports
                        )));
                    }
                }
            }
        };

        info!("opening serial link on {}", serial);

        let serial_stream = utils::stream::build_serial_stream(serial.clone(), None, None, None)
            .expect("no radio found");
        let config_id = utils::generate_rand_id();
        let stream_api = StreamApi::new();
        let (mesh_in_rx, stream_api) = stream_api.connect(serial_stream).await;
        let stream_api = stream_api.configure(config_id).await?;

        // Channel used for receiving outgoing messages from the client to be sent to the mesh
        let (client_in_tx, client_in_rx) = mpsc::channel::<LinkMessage>(100);

        // Channel used for sending (defragmented) incoming messages to the client
        let (client_out_tx, client_out_rx) = mpsc::channel::<LinkMessage>(100);

        let stop_signal = Arc::new(Notify::new());
        let slinkref = Arc::new(Mutex::new(SerialLink::new(
            stream_api,
            client_out_tx,
            stop_signal.clone(),
        )));

        SerialLink::start_mesh_listener(slinkref.clone(), mesh_in_rx, stop_signal.clone())?;

        SerialLink::start_client_listener(slinkref.clone(), client_in_rx, stop_signal.clone())?;

        Ok((slinkref, client_in_tx, client_out_rx))
    }

    fn start_mesh_listener(
        linkref: SerialLinkRef,
        mut mesh_in_rx: mpsc::UnboundedReceiver<FromRadio>,
        stop_signal: Arc<Notify>,
    ) -> LinkResult<()> {
        task::spawn(async move {
            info!("mesh_listener starting");
            loop {
                tokio::select! {
                    Some(packet) = mesh_in_rx.recv() => {
                        Self::handle_packet(&linkref, packet).await;
                    },
                    _ = stop_signal.notified() => {
                        break;
                    }
                }
            }
            info!("mesh_listener finished");
        });
        Ok(())
    }

    // Handle incoming packets
    async fn handle_packet(linkref: &SerialLinkRef, packet: FromRadio) {
        match packet.payload_variant {
            Some(from_radio::PayloadVariant::MyInfo(myinfo)) => {
                linkref.lock().await.set_my_node_num(myinfo.my_node_num);
            }
            Some(from_radio::PayloadVariant::Packet(mesh_packet)) => {
                Self::handle_mesh_packet(linkref, mesh_packet).await;
            }
            _ => {} // Ignore other variants
        }
    }

    // Handle mesh packets
    async fn handle_mesh_packet(linkref: &SerialLinkRef, mesh_packet: MeshPacket) {
        if let Some(mesh_packet::PayloadVariant::Decoded(ref decoded)) = mesh_packet.payload_variant
        {
            if decoded.portnum() == PortNum::PrivateApp {
                match Self::decode_link_frame(decoded.payload.clone()) {
                    Ok(link_frame) => Self::process_link_frame(linkref, link_frame).await,
                    Err(err) => error!("Failed to decode LinkFrame: {:?}", err),
                }
            }
        }
    }

    // Decode the LinkFrame
    fn decode_link_frame(payload: Vec<u8>) -> Result<LinkFrame, prost::DecodeError> {
        debug!("received LinkFrame, encoded sz: {}", payload.len());
        LinkFrame::decode(&*payload)
    }

    // Process the LinkFrame
    async fn process_link_frame(linkref: &SerialLinkRef, link_frame: LinkFrame) {
        match link_frame.payload {
            Some(Payload::Complete(linkmsg)) => {
                Self::handle_complete(linkref, linkmsg).await;
            }
            Some(Payload::Fragment(linkfrag)) => {
                Self::handle_fragment(linkref, linkfrag).await;
            }
            None => {
                warn!("LinkFrame payload is missing");
            }
        }
    }

    // Handle unfragmented messages
    async fn handle_complete(linkref: &SerialLinkRef, linkmsg: LinkMsg) {
        debug!(
            "received complete LinkMsg w/ payload sz: {}",
            linkmsg.data.len()
        );
        let link = linkref.lock().await;
        if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
            error!("failed to send message: {}", err);
        }
    }

    // Handle fragmented messages (stub for now)
    async fn handle_fragment(linkref: &SerialLinkRef, frag: LinkFrag) {
        debug!(
            "received LinkFrag {}: {}/{} payload sz: {}",
            MsgId(frag.msgid),
            frag.fragndx,
            frag.numfrag,
            frag.data.len()
        );
        let inbound = true;
        let mut link = linkref.lock().await;
        if let Some(linkmsg) = link.fragcache.add_fragment(&frag, inbound) {
            debug!("completed LinkFrag {}", MsgId(frag.msgid));
            if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
                error!("failed to send message: {}", err);
            }
        }
    }

    fn start_client_listener(
        linkref: SerialLinkRef,
        mut client_in_rx: mpsc::Receiver<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) -> LinkResult<()> {
        task::spawn(async move {
            info!("client_listener starting");
            loop {
                tokio::select! {
                    Some(msg) = client_in_rx.recv() => {
                        if let Err(err) = Self::send_link_message(linkref.clone(), msg).await {
                            error!("send failed: {:?}", err);
                        }
                    },
                    _ = stop_signal.notified() => {
                        break;
                    }
                }
            }
            info!("client_listener finished");
        });
        Ok(())
    }

    async fn send_link_message(linkref: SerialLinkRef, msg: LinkMessage) -> LinkResult<()> {
        if msg.data.len() > LINK_FRAG_THRESH {
            Ok(Self::send_fragments(linkref, msg).await?)
        } else {
            Ok(Self::send_complete(linkref, msg).await?)
        }
    }

    async fn send_complete(linkref: SerialLinkRef, msg: LinkMessage) -> LinkResult<()> {
        let link_msg = LinkMsg {
            data: msg.data.clone(),
        };
        let link_frame = LinkFrame {
            magic: LINK_MAGIC,
            version: LINK_VERSION,
            payload: Some(Payload::Complete(link_msg)),
        };
        debug!("sending complete LinkMsg w/ payload sz: {}", msg.data.len());
        Self::send_link_frame(linkref, link_frame).await
    }

    fn compute_message_id(data: &[u8]) -> MsgId {
        let hash = Sha256::digest(data);
        let bytes = &hash[..8]; // Take the first 8 bytes
        MsgId(u64::from_be_bytes(
            bytes.try_into().expect("Slice has incorrect length"),
        ))
    }

    async fn send_fragments(linkref: SerialLinkRef, msg: LinkMessage) -> LinkResult<()> {
        let msgid = Self::compute_message_id(&msg.data);
        let data = &msg.data;
        let numfrag: u32 = msg.data.len().div_ceil(LINK_FRAG_THRESH) as u32;
        for (fragndx, chunk) in (0_u32..).zip(data.chunks(LINK_FRAG_THRESH)) {
            let link_frag = LinkFrag {
                msgid: msgid.0,
                numfrag,
                fragndx,
                data: chunk.to_vec(),
            };
            let inbound = false;
            linkref
                .lock()
                .await
                .fragcache
                .add_fragment(&link_frag, inbound);
            let link_frame = LinkFrame {
                magic: LINK_MAGIC,
                version: LINK_VERSION,
                payload: Some(Payload::Fragment(link_frag)),
            };
            debug!(
                "sending LinkFrag {}: {}/{} payload sz: {}",
                msgid,
                fragndx,
                numfrag,
                chunk.len()
            );
            Self::send_link_frame(linkref.clone(), link_frame).await?;
        }
        Ok(())
    }

    async fn send_link_frame(linkref: SerialLinkRef, frame: LinkFrame) -> LinkResult<()> {
        // Serialize the LinkFrame into bytes
        let mut buffer = Vec::new();
        frame.encode(&mut buffer).map_err(|err| {
            LinkError::internal_error(format!("send_link_message: encode error: {:?}", err))
        })?;

        let mut link = linkref.lock().await;

        debug!("sending LinkFrame, encoded sz: {}", buffer.len());
        let mut router = LinkPacketRouter {
            my_id: link.my_node_num.into(),
        };
        let port_num = PortNum::PrivateApp;
        let destination = PacketDestination::Broadcast;
        let channel = 0.into();
        let want_ack = false;
        let want_response = false;
        let echo_response = false;
        let reply_id: Option<u32> = None;
        let emoji: Option<u32> = None;
        if let Err(err) = link
            .stream_api
            .send_mesh_packet(
                &mut router,
                buffer.into(),
                port_num,
                destination,
                channel,
                want_ack,
                want_response,
                echo_response,
                reply_id,
                emoji,
            )
            .await
        {
            error!("send_mesh_packet failed {:?}", err);
        }

        // don't send packets back to back
        sleep(Duration::from_secs(2)).await;

        Ok(())
    }
}

#[async_trait]
impl MeshtasticLink for SerialLink {
    // maybe not needed?
}

struct LinkPacketRouter {
    my_id: NodeId,
}

impl PacketRouter<(), LinkError> for LinkPacketRouter {
    fn handle_packet_from_radio(&mut self, packet: FromRadio) -> Result<(), LinkError> {
        dbg!(packet);
        Ok(())
    }

    fn handle_mesh_packet(&mut self, packet: MeshPacket) -> Result<(), LinkError> {
        dbg!(packet);
        Ok(())
    }

    fn source_node_id(&self) -> NodeId {
        self.my_id
    }
}
