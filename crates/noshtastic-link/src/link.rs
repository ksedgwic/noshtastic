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
    io::Cursor,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, Notify},
    task,
    time::{self, Duration, Instant},
};

use crate::{
    outgoing::Outgoing, proto::LinkNeed, FragmentCache, LinkConfig, LinkFrag, LinkFrame,
    LinkInMessage, LinkInfo, LinkMsg, LinkOutMessage, LinkOutOptionsBuilder, LinkOutPayload,
    LinkResult, MsgId, Payload, Priority,
};

const LINK_VERSION: u32 = 1;
const LINK_MAGIC: u32 = 0x48534F4E; // 'NOSH'
const LINK_FRAG_THRESH: usize = 200;
const LINK_STALE_SECS: i64 = 300; // messages older than this are considered stale
const LINK_READY_HOLDOFF_SECS: u64 = 5; // wait this long after settled
const LINK_INFO_SECS: u64 = 60; // send link info periodically
const LINK_SEND_NEED_SECS: u64 = 180; // send request for needed msgs fragments

pub struct Link {
    pub(crate) stream_api: ConnectedStreamApi,
    client_out_tx: mpsc::UnboundedSender<LinkInMessage>,
    _stop_signal: Arc<Notify>,
    pub(crate) my_node_num: u32,
    fragcache: FragmentCache,
    outgoing: Outgoing,
    ready_deadline: Instant,
    declared_ready: bool,
}
pub type LinkRef = Arc<tokio::sync::Mutex<Link>>;

impl Link {
    pub(crate) fn new(
        link_config: &LinkConfig,
        stream_api: ConnectedStreamApi,
        client_out_tx: mpsc::UnboundedSender<LinkInMessage>,
        stop_signal: Arc<Notify>,
    ) -> Self {
        Link {
            stream_api,
            client_out_tx,
            _stop_signal: stop_signal,
            my_node_num: 0,
            fragcache: FragmentCache::new(),
            outgoing: Outgoing::new(link_config),
            ready_deadline: Instant::now() + Duration::from_secs(LINK_READY_HOLDOFF_SECS),
            declared_ready: false,
        }
    }

    fn set_my_node_num(&mut self, my_node_num: u32) {
        info!("setting my_node_num: {}", my_node_num);
        self.my_node_num = my_node_num;
    }

    pub(crate) async fn start(
        linkref: &LinkRef,
        mesh_in_rx: PacketReceiver,
        client_in_rx: mpsc::UnboundedReceiver<LinkOutMessage>,
        stop_signal: Arc<Notify>,
    ) -> LinkResult<()> {
        // Listen for incoming messages from the radio
        Link::start_mesh_listener(linkref.clone(), mesh_in_rx, stop_signal.clone())?;

        // Listen for new outgoing messages from the client
        Link::start_client_listener(linkref.clone(), client_in_rx, stop_signal.clone())?;

        // Start the regulator which sends all outgoing packets
        linkref
            .lock()
            .await
            .outgoing
            .start_regulator(linkref.clone())?;

        // Check for link ready
        Link::start_check_ready(linkref.clone(), stop_signal.clone())?;

        // Report LinkInfo (this initiates negentropy sync when
        // appropriate, and handles the fragcache retries)
        Link::start_link_info(linkref.clone(), stop_signal.clone())?;

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

    fn start_check_ready(linkref: LinkRef, stop_signal: Arc<Notify>) -> LinkResult<()> {
        let mut interval = time::interval(Duration::from_secs(1));
        task::spawn(async move {
            info!("check_ready starting");
            loop {
                tokio::select! {
                    _ = stop_signal.notified() => {
                        break;
                    }
                    _ = interval.tick() => {
                        if Self::maybe_declare_ready(linkref.clone()).await {
                            break;
                        }
                    }
                }
            }
            info!("check_ready finished");
        });
        Ok(())
    }

    fn start_link_info(linkref: LinkRef, stop_signal: Arc<Notify>) -> LinkResult<()> {
        let mut info_iv = time::interval(Duration::from_secs(LINK_INFO_SECS));
        let mut need_iv = time::interval(Duration::from_secs(LINK_SEND_NEED_SECS));
        task::spawn(async move {
            info!("check_ready starting");
            loop {
                tokio::select! {
                    _ = stop_signal.notified() => break,

                    // every LINK_INFO_SECS
                    _ = info_iv.tick() => {
                        let link = linkref.lock().await;
                        let qlen = link.outgoing.qlen().await;
                        let info = LinkInfo { qlen };
                        let _ = link.client_out_tx.send(LinkInMessage::Info(info));
                    },

                    // every LINK_SEND_NEED_SECS
                    _ = need_iv.tick() => {
                        Self::send_fragment_retries(linkref.clone()).await;
                    }
                }
            }
            info!("check_ready finished");
        });
        Ok(())
    }

    async fn maybe_declare_ready(linkref: LinkRef) -> bool {
        let mut link = linkref.lock().await;
        if link.declared_ready {
            return true;
        }
        if Instant::now() >= link.ready_deadline {
            // Mark as ready!
            if let Err(err) = link.client_out_tx.send(LinkInMessage::Ready) {
                error!("maybe_declare_ready: failed to send message: {}", err);
            }
            link.declared_ready = true;

            // Follow with an immediate NodeInfo so we can initiate right away
            let qlen = link.outgoing.qlen().await;
            let info = LinkInfo { qlen };
            if let Err(err) = link.client_out_tx.send(LinkInMessage::Info(info)) {
                error!("send link info: failed to send message: {}", err);
            }

            return true;
        }
        false
    }

    async fn send_fragment_retries(linkref: LinkRef) {
        let mut link = linkref.lock().await;

        // collect all overdue missing entries
        let missing_list = link.fragcache.overdue_missing();
        if missing_list.is_empty() {
            return;
        }

        // build a single LinkNeed containing them all
        let need = LinkNeed {
            missing: missing_list.clone(), // assume Vec<LinkMissing>
        };

        let link_frame = LinkFrame {
            magic: LINK_MAGIC,
            version: LINK_VERSION,
            payload: Some(Payload::Need(need)),
        };

        // use a synthetic MsgId for the batch‐request
        let mut buf = Vec::new();
        link_frame.encode(&mut buf).unwrap();
        let msgid = MsgId::from(&buf[..]);

        let outcome = link
            .outgoing
            .enqueue(
                msgid,
                link_frame,
                LinkOutOptionsBuilder::new()
                    .priority(Priority::High)
                    .build(),
            )
            .await;

        info!(
            "queued LinkNeed retry ({} messages) => {}",
            missing_list.len(),
            outcome
        );
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

    async fn postpone_ready(linkref: &LinkRef) {
        let mut link = linkref.lock().await;
        link.ready_deadline = Instant::now() + Duration::from_secs(LINK_READY_HOLDOFF_SECS);
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
                info!("skipping stale message: {} secs old", age);
                Self::postpone_ready(linkref).await;
                return;
            }
            if decoded.portnum() == PortNum::PrivateApp {
                debug!("received LinkFrame, sz: {}", decoded.payload.len(),);

                // Unescape a pathological sequence (see noshtastic#15)
                let payload = crate::unescape94c3(&decoded.payload).expect("unescaped");

                match LinkFrame::decode(Cursor::new(payload)) {
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
            Some(Payload::Need(linkneed)) => {
                Self::handle_need(linkref, linkneed).await;
            }
            None => {
                warn!("LinkFrame payload is missing");
            }
        }
    }

    // Handle unfragmented messages
    async fn handle_complete(linkref: &LinkRef, linkmsg: LinkMsg) {
        let msgid = MsgId::new(linkmsg.msgid, None);
        info!(
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
                LinkOutOptionsBuilder::new()
                    .priority(Priority::High)
                    .build(),
            )
            .await;
        link.outgoing
            .cancel(
                msgid,
                LinkOutOptionsBuilder::new()
                    .priority(Priority::Normal)
                    .build(),
            )
            .await;

        // send the message to the client
        if let Err(err) = link.client_out_tx.send(LinkInMessage::from(linkmsg)) {
            error!("failed to send message: {}", err);
        }
    }

    // Handle fragmented messages
    async fn handle_fragment(linkref: &LinkRef, frag: LinkFrag) {
        let msgid = MsgId::new(frag.msgid, Some(frag.fragndx));
        info!(
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
                LinkOutOptionsBuilder::new()
                    .priority(Priority::High)
                    .build(),
            )
            .await;
        link.outgoing
            .cancel(
                msgid,
                LinkOutOptionsBuilder::new()
                    .priority(Priority::Normal)
                    .build(),
            )
            .await;

        // add the fragment and if completed send the message to the client
        if let Some(linkmsg) = link.fragcache.add_fragment(&frag, inbound) {
            info!("completed LinkFrag {}", msgid);
            if let Err(err) = link.client_out_tx.send(LinkInMessage::from(linkmsg)) {
                error!("failed to send message: {}", err);
            }
        }
    }

    // Handle batch‐requests for missing fragments
    async fn handle_need(linkref: &LinkRef, need: LinkNeed) {
        info!("received LinkNeed ({} entries)", need.missing.len());
        let mut link = linkref.lock().await;

        for missing in need.missing {
            // base ID for this msg
            let base = missing.msgid;
            let msgid_base = MsgId::new(base, None);
            info!(
                "processing missing for {}: {:?}",
                msgid_base, missing.fragndx
            );

            // ask cache for the actual fragments to resend
            for frag in link.fragcache.fulfill_missing(missing.clone()) {
                let fragndx = frag.fragndx;
                let numfrag = frag.numfrag;
                let fraglen = frag.data.len();
                let rotoff = frag.rotoff;

                // build and enqueue the retry‐fragment
                let link_frame = LinkFrame {
                    magic: LINK_MAGIC,
                    version: LINK_VERSION,
                    payload: Some(Payload::Fragment(frag.clone())),
                };
                let retry_id = MsgId::new(base, Some(fragndx));

                let outcome = link
                    .outgoing
                    .enqueue(
                        retry_id,
                        link_frame,
                        LinkOutOptionsBuilder::new()
                            .priority(Priority::High)
                            .build(),
                    )
                    .await;

                info!(
                    "requeueing LinkFrag {}/{}, rotoff: {}, sz: {} => {}",
                    retry_id, numfrag, rotoff, fraglen, outcome
                );
            }
        }
    }

    fn start_client_listener(
        linkref: LinkRef,
        mut client_in_rx: mpsc::UnboundedReceiver<LinkOutMessage>,
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

    async fn send_link_message(linkref: LinkRef, msg: LinkOutMessage) -> LinkResult<()> {
        match msg {
            LinkOutMessage::Payload(payload) => {
                if payload.data.len() > LINK_FRAG_THRESH {
                    Ok(Self::send_fragments(linkref, payload).await?)
                } else {
                    Ok(Self::send_complete(linkref, payload).await?)
                }
            }
        }
    }

    async fn send_complete(linkref: LinkRef, payload: LinkOutPayload) -> LinkResult<()> {
        let link_msg = LinkMsg {
            msgid: payload.msgid.base,
            data: payload.data.clone(),
        };
        let link_frame = LinkFrame {
            magic: LINK_MAGIC,
            version: LINK_VERSION,
            payload: Some(Payload::Complete(link_msg)),
        };
        let mut link = linkref.lock().await;
        let outcome = link
            .outgoing
            .enqueue(payload.msgid, link_frame, payload.options)
            .await;
        info!(
            "queueing complete LinkMsg {} w/ payload sz: {} => {}",
            payload.msgid,
            payload.data.len(),
            outcome
        );
        Ok(())
    }

    async fn send_fragments(linkref: LinkRef, payload: LinkOutPayload) -> LinkResult<()> {
        let data = &payload.data;
        let numfrag: u32 = payload.data.len().div_ceil(LINK_FRAG_THRESH) as u32;
        let rotoff = 0;
        let mut link = linkref.lock().await;
        for (fragndx, chunk) in (0_u32..).zip(data.chunks(LINK_FRAG_THRESH)) {
            let link_frag = LinkFrag {
                msgid: payload.msgid.base,
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

            let msgid = MsgId::new(payload.msgid.base, Some(fragndx));
            let outcome = link
                .outgoing
                .enqueue(msgid, link_frame, payload.options.clone())
                .await;
            info!(
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
