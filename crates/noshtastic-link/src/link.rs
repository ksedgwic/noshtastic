// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use meshtastic::{
    api::ConnectedStreamApi,
    packet::PacketReceiver,
    protobufs::{from_radio, mesh_packet, FromRadio, MeshPacket, PortNum},
};
use prost::Message;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, Notify},
    task,
    time::{self, Duration},
};

use crate::{
    outgoing::Outgoing, proto::LinkMissing, FragmentCache, LinkFrag, LinkFrame, LinkMessage,
    LinkMsg, LinkOptionsBuilder, LinkResult, MsgId, Payload, Priority,
};

const LINK_VERSION: u32 = 1;
const LINK_MAGIC: u32 = 0x48534F4E; // 'NOSH'
const LINK_FRAG_THRESH: usize = 200;
const LINK_STALE_SECS: i64 = 300; // messages older than this are considered stale

pub struct Link {
    pub(crate) stream_api: ConnectedStreamApi,
    client_out_tx: mpsc::Sender<LinkMessage>,
    _stop_signal: Arc<Notify>,
    pub(crate) my_node_num: u32,
    fragcache: FragmentCache,
    outgoing: Outgoing,
}
pub type LinkRef = Arc<tokio::sync::Mutex<Link>>;

impl Link {
    pub(crate) fn new(
        stream_api: ConnectedStreamApi,
        client_out_tx: mpsc::Sender<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) -> Self {
        Link {
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

    pub(crate) async fn start(
        linkref: &LinkRef,
        mesh_in_rx: PacketReceiver,
        client_in_rx: mpsc::Receiver<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) -> LinkResult<()> {
        // Listen for incoming messages from the radio
        Link::start_mesh_listener(linkref.clone(), mesh_in_rx, stop_signal.clone())?;

        // Listen for new outgoing messages from the client
        Link::start_client_listener(linkref.clone(), client_in_rx, stop_signal.clone())?;

        // Periodically request retransmission of missing fragments
        Link::start_fragchache_periodic(linkref.clone(), stop_signal.clone())?;

        // Start the regulator which sends all outgoing packets
        linkref
            .lock()
            .await
            .outgoing
            .start_regulator(linkref.clone())?;

        Ok(())
    }

    fn start_mesh_listener(
        linkref: LinkRef,
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

    fn start_fragchache_periodic(linkref: LinkRef, stop_signal: Arc<Notify>) -> LinkResult<()> {
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

    async fn send_fragment_retries(linkref: LinkRef) {
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
                        .action(crate::Action::Drop)
                        .priority(Priority::High)
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
    async fn handle_packet(linkref: &LinkRef, packet: FromRadio) {
        match packet.payload_variant {
            Some(from_radio::PayloadVariant::MyInfo(myinfo)) => {
                linkref.lock().await.set_my_node_num(myinfo.my_node_num);
            }
            Some(from_radio::PayloadVariant::Packet(mesh_packet)) => {
                Self::handle_mesh_packet(linkref, mesh_packet).await;
            }
            other => {
                log::debug!("fromradio: {:?}", other);
            }
        }
    }

    // Handle mesh packets
    async fn handle_mesh_packet(linkref: &LinkRef, mesh_packet: MeshPacket) {
        if let Some(mesh_packet::PayloadVariant::Decoded(ref decoded)) = mesh_packet.payload_variant
        {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64;
            let age = now - mesh_packet.rx_time as i64;
            if age > LINK_STALE_SECS {
                debug!("skipping stale message: {} secs old", age);
                return;
            }
            if decoded.portnum() == PortNum::PrivateApp {
                debug!("received LinkFrame, sz: {}", decoded.payload.len(),);
                match LinkFrame::decode(&*decoded.payload) {
                    Ok(link_frame) => Self::handle_link_frame(linkref, link_frame).await,
                    Err(err) => error!("Failed to decode LinkFrame: {:?}", err),
                }
            }
        }
    }

    // Process the LinkFrame
    async fn handle_link_frame(linkref: &LinkRef, linkframe: LinkFrame) {
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
    async fn handle_complete(linkref: &LinkRef, linkmsg: LinkMsg) {
        let msgid = MsgId::new(linkmsg.msgid, None);
        debug!(
            "received complete LinkMsg {} w/ payload sz: {}",
            msgid,
            linkmsg.data.len()
        );

        let mut link = linkref.lock().await;

        // cancel any outgoing sends of this message that are in our
        // queues (another node beat us to it ...)
        link.outgoing
            .cancel(
                msgid,
                LinkOptionsBuilder::new().priority(Priority::High).build(),
            )
            .await;
        link.outgoing
            .cancel(
                msgid,
                LinkOptionsBuilder::new().priority(Priority::Normal).build(),
            )
            .await;

        // send the message to the client
        if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
            error!("failed to send message: {}", err);
        }
    }

    // Handle fragmented messages
    async fn handle_fragment(linkref: &LinkRef, frag: LinkFrag) {
        let msgid = MsgId::new(frag.msgid, Some(frag.fragndx));
        debug!(
            "received LinkFrag {}/{}, rotoff: {}, payload sz: {}",
            msgid,
            frag.numfrag,
            frag.rotoff,
            frag.data.len()
        );

        let inbound = true;
        let mut link = linkref.lock().await;

        // cancel any outgoing sends of this fragment that are in our
        // queues (another node beat us to it ...)
        link.outgoing
            .cancel(
                msgid,
                LinkOptionsBuilder::new().priority(Priority::High).build(),
            )
            .await;
        link.outgoing
            .cancel(
                msgid,
                LinkOptionsBuilder::new().priority(Priority::Normal).build(),
            )
            .await;

        // add the fragment and if completed send the message to the client
        if let Some(linkmsg) = link.fragcache.add_fragment(&frag, inbound) {
            debug!("completed LinkFrag {}", msgid);
            if let Err(err) = link.client_out_tx.send(LinkMessage::from(linkmsg)).await {
                error!("failed to send message: {}", err);
            }
        }
    }

    // Handle requests for missing fragments
    async fn handle_missing(linkref: &LinkRef, missing: LinkMissing) {
        let msgid = MsgId::new(missing.msgid, None);
        debug!("received LinkMissing {}: {:?}", msgid, missing.fragndx);
        let mut link = linkref.lock().await;
        for frag in link.fragcache.fulfill_missing(missing) {
            let fragndx = frag.fragndx;
            let numfrag = frag.numfrag;
            let fraglen = frag.data.len();
            let rotoff = frag.rotoff;
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
                    LinkOptionsBuilder::new().priority(Priority::High).build(),
                )
                .await;
            debug!(
                "requeueing LinkFrag {}/{}, rotoff: {}, payload sz: {} => {}",
                msgid, numfrag, rotoff, fraglen, outcome
            );
        }
    }

    fn start_client_listener(
        linkref: LinkRef,
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

    async fn send_link_message(linkref: LinkRef, msg: LinkMessage) -> LinkResult<()> {
        if msg.data.len() > LINK_FRAG_THRESH {
            Ok(Self::send_fragments(linkref, msg).await?)
        } else {
            Ok(Self::send_complete(linkref, msg).await?)
        }
    }

    async fn send_complete(linkref: LinkRef, msg: LinkMessage) -> LinkResult<()> {
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

    async fn send_fragments(linkref: LinkRef, msg: LinkMessage) -> LinkResult<()> {
        let data = &msg.data;
        let numfrag: u32 = msg.data.len().div_ceil(LINK_FRAG_THRESH) as u32;
        let rotoff = 0;
        let mut link = linkref.lock().await;
        for (fragndx, chunk) in (0_u32..).zip(data.chunks(LINK_FRAG_THRESH)) {
            let link_frag = LinkFrag {
                msgid: msg.msgid.base,
                numfrag,
                fragndx,
                rotoff,
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
