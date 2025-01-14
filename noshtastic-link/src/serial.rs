// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use async_trait::async_trait;
use log::*;
use meshtastic::{
    api::{ConnectedStreamApi, StreamApi},
    protobufs::{from_radio, mesh_packet, FromRadio, MeshPacket, PortNum},
    utils,
};
use prost::Message;
use std::{process, sync::Arc};
use tokio::{
    sync::{mpsc, Mutex, Notify},
    task,
    time::{self, Duration},
};

use crate::{
    outgoing::Outgoing, proto::LinkMissing, Action, FragmentCache, LinkError, LinkFrag, LinkFrame,
    LinkMessage, LinkMsg, LinkOptionsBuilder, LinkRef, LinkResult, MeshtasticLink, MsgId, Payload,
    Priority,
};

const LINK_VERSION: u32 = 1;
const LINK_MAGIC: u32 = 0x48534F4E; // 'NOSH'
const LINK_FRAG_THRESH: usize = 200;

#[derive(Debug)]
pub(crate) struct SerialLink {
    pub(crate) stream_api: ConnectedStreamApi,
    client_out_tx: mpsc::Sender<LinkMessage>,
    _stop_signal: Arc<Notify>,
    pub(crate) my_node_num: u32,
    fragcache: FragmentCache,
    outgoing: Outgoing,
}
pub(crate) type SerialLinkRef = Arc<tokio::sync::Mutex<SerialLink>>;

impl SerialLink {
    pub(crate) fn new(
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
            outgoing: Outgoing::new(),
        }
    }

    fn set_my_node_num(&mut self, my_node_num: u32) {
        info!("setting my_node_num: {}", my_node_num);
        self.my_node_num = my_node_num;
    }

    pub(crate) async fn create_serial_link(
        maybe_serial: &Option<String>,
        stop_signal: Arc<Notify>,
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
            .map_err(|err| {
                error!("failed to open {}: {:?}", serial, err);
                info!(
                    "available ports: {:?}. Use --serial to specify.",
                    utils::stream::available_serial_ports().expect("available_serial_ports")
                );
                process::exit(1);
            })
            .unwrap();

        let config_id = utils::generate_rand_id();
        let stream_api = StreamApi::new();
        let (mesh_in_rx, stream_api) = stream_api.connect(serial_stream).await;
        let stream_api = stream_api.configure(config_id).await?;

        // Channel used for receiving outgoing messages from the client to be sent to the mesh
        let (client_in_tx, client_in_rx) = mpsc::channel::<LinkMessage>(100);

        // Channel used for sending (defragmented) incoming messages to the client
        let (client_out_tx, client_out_rx) = mpsc::channel::<LinkMessage>(100);

        let slinkref = Arc::new(Mutex::new(SerialLink::new(
            stream_api,
            client_out_tx,
            stop_signal.clone(),
        )));

        SerialLink::start_mesh_listener(slinkref.clone(), mesh_in_rx, stop_signal.clone())?;
        SerialLink::start_client_listener(slinkref.clone(), client_in_rx, stop_signal.clone())?;
        SerialLink::start_fragchache_periodic(slinkref.clone(), stop_signal.clone())?;
        slinkref
            .lock()
            .await
            .outgoing
            .start_regulator(slinkref.clone())?;

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

    fn start_fragchache_periodic(
        linkref: SerialLinkRef,
        stop_signal: Arc<Notify>,
    ) -> LinkResult<()> {
        let mut interval = time::interval(Duration::from_secs(5));
        task::spawn(async move {
            info!("fragcache_periodic starting");
            loop {
                tokio::select! {
                    _ = stop_signal.notified() => {
                        break;
                    }
                    _ = interval.tick() => {
                        Self::send_fragment_retries(linkref.clone()).await;
                    }
                }
            }
            info!("fragcache_periodic finished");
        });
        Ok(())
    }

    async fn send_fragment_retries(linkref: SerialLinkRef) {
        let mut link = linkref.lock().await;
        for missing in link.fragcache.overdue_missing() {
            let msgid = MsgId::new(missing.msgid, None);
            let fragndx = missing.fragndx.clone();
            let link_frame = LinkFrame {
                magic: LINK_MAGIC,
                version: LINK_VERSION,
                payload: Some(Payload::Missing(missing)),
            };

            let outcome = link
                .outgoing
                .enqueue(
                    msgid,
                    link_frame,
                    LinkOptionsBuilder::new()
                        .priority(Priority::High)
                        .action(Action::Replace)
                        .build(),
                )
                .await;
            debug!(
                "queueing LinkMissing {}: {:?} => {}",
                msgid, fragndx, outcome
            );
        }
    }

    // Handle incoming packets from the radio
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
                debug!("received LinkFrame, sz: {}", decoded.payload.len());
                match LinkFrame::decode(&*decoded.payload) {
                    Ok(link_frame) => Self::handle_link_frame(linkref, link_frame).await,
                    Err(err) => error!("Failed to decode LinkFrame: {:?}", err),
                }
            }
        }
    }

    // Process the LinkFrame
    async fn handle_link_frame(linkref: &SerialLinkRef, linkframe: LinkFrame) {
        match linkframe.payload {
            Some(Payload::Complete(linkmsg)) => {
                Self::handle_complete(linkref, linkmsg).await;
            }
            Some(Payload::Fragment(linkfrag)) => {
                Self::handle_fragment(linkref, linkfrag).await;
            }
            Some(Payload::Missing(linkmissing)) => {
                Self::handle_missing(linkref, linkmissing).await;
            }
            None => {
                warn!("LinkFrame payload is missing");
            }
        }
    }

    // Handle unfragmented messages
    async fn handle_complete(linkref: &SerialLinkRef, linkmsg: LinkMsg) {
        debug!(
            "received complete LinkMsg {} w/ payload sz: {}",
            MsgId::new(linkmsg.msgid, None),
            linkmsg.data.len()
        );
        let link = linkref.lock().await;
        if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
            error!("failed to send message: {}", err);
        }
    }

    // Handle fragmented messages
    async fn handle_fragment(linkref: &SerialLinkRef, frag: LinkFrag) {
        let msgid = MsgId::new(frag.msgid, Some(frag.fragndx));
        debug!(
            "received LinkFrag {}/{} payload sz: {}",
            msgid,
            frag.numfrag,
            frag.data.len()
        );
        let inbound = true;
        let mut link = linkref.lock().await;
        if let Some(linkmsg) = link.fragcache.add_fragment(&frag, inbound) {
            debug!("completed LinkFrag {}", msgid);
            if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
                error!("failed to send message: {}", err);
            }
        }
    }

    // Handle requests for missing fragments
    async fn handle_missing(linkref: &SerialLinkRef, missing: LinkMissing) {
        let msgid = MsgId::new(missing.msgid, None);
        debug!("received LinkMissing {}: {:?}", msgid, missing.fragndx);
        let mut link = linkref.lock().await;
        for frag in link.fragcache.fulfill_missing(missing) {
            let fragndx = frag.fragndx;
            let numfrag = frag.numfrag;
            let fraglen = frag.data.len();
            let link_frame = LinkFrame {
                magic: LINK_MAGIC,
                version: LINK_VERSION,
                payload: Some(Payload::Fragment(frag)),
            };
            let msgid = MsgId::new(msgid.base, Some(fragndx));
            let outcome = link
                .outgoing
                .enqueue(
                    msgid,
                    link_frame,
                    LinkOptionsBuilder::new()
                        .priority(Priority::High)
                        .action(Action::Replace)
                        .build(),
                )
                .await;
            debug!(
                "requeueing LinkFrag {}/{} payload sz: {} => {}",
                msgid, numfrag, fraglen, outcome
            );
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
            msgid: msg.msgid.base,
            data: msg.data.clone(),
        };
        let link_frame = LinkFrame {
            magic: LINK_MAGIC,
            version: LINK_VERSION,
            payload: Some(Payload::Complete(link_msg)),
        };
        let mut link = linkref.lock().await;
        let outcome = link
            .outgoing
            .enqueue(msg.msgid, link_frame, msg.options)
            .await;
        debug!(
            "queueing complete LinkMsg {} w/ payload sz: {} => {}",
            msg.msgid,
            msg.data.len(),
            outcome
        );
        Ok(())
    }

    async fn send_fragments(linkref: SerialLinkRef, msg: LinkMessage) -> LinkResult<()> {
        let data = &msg.data;
        let numfrag: u32 = msg.data.len().div_ceil(LINK_FRAG_THRESH) as u32;
        let mut link = linkref.lock().await;
        for (fragndx, chunk) in (0_u32..).zip(data.chunks(LINK_FRAG_THRESH)) {
            let link_frag = LinkFrag {
                msgid: msg.msgid.base,
                numfrag,
                fragndx,
                data: chunk.to_vec(),
            };
            let inbound = false;
            link.fragcache.add_fragment(&link_frag, inbound);
            let link_frame = LinkFrame {
                magic: LINK_MAGIC,
                version: LINK_VERSION,
                payload: Some(Payload::Fragment(link_frag)),
            };

            // If you are trying to induce packet failure to test retries this
            // is a good spot to just `continue` w/o sending the packet ...

            let msgid = MsgId::new(msg.msgid.base, Some(fragndx));
            let outcome = link
                .outgoing
                .enqueue(msgid, link_frame, msg.options.clone())
                .await;
            debug!(
                "queueing LinkFrag {}/{} payload sz: {} => {}",
                msgid,
                numfrag,
                chunk.len(),
                outcome
            );
        }
        Ok(())
    }
}

#[async_trait]
impl MeshtasticLink for SerialLink {
    // maybe not needed?
}
