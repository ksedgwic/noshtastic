// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use meshtastic::{
    packet::{PacketDestination, PacketRouter},
    protobufs::{FromRadio, MeshPacket, PortNum},
    types::NodeId,
};
use prost::Message;
use std::{collections::VecDeque, sync::Arc};
use tokio::{
    sync::{mpsc, Mutex},
    task,
    time::{sleep, Duration},
};

use crate::{
    Action, LinkConfig, LinkError, LinkFrame, LinkOptions, LinkRef, LinkResult, MsgId, Priority,
};

#[derive(Debug)]
struct OutgoingPolicy {
    tx_rate_limit_msec: u64,
    outgoing_queue_max: usize,
}

impl OutgoingPolicy {
    fn from_link_config(link_cfg: &LinkConfig) -> Self {
        // baseline data rate for “defaults” = 3.52 kbps (MEDIUM_FAST)
        const BASE_DATA_KBPS: f32 = 3.52;
        const TX_RATE_LIMIT_MSEC: f32 = 10_000.0;
        const OUTGOING_QUEUE_MAX: f32 = 100.0;

        // scale factors
        let rate_scale = BASE_DATA_KBPS / link_cfg.data_kbps;
        let queue_scale = link_cfg.data_kbps / BASE_DATA_KBPS;

        // scale to match other data rates
        let tx_rate_limit_msec = (TX_RATE_LIMIT_MSEC * rate_scale).max(1.0).round() as u64;
        let outgoing_queue_max = (OUTGOING_QUEUE_MAX * queue_scale).max(1.0).round() as usize;

        OutgoingPolicy {
            tx_rate_limit_msec,
            outgoing_queue_max,
        }
    }
}

#[derive(Debug)]
struct Queues {
    low: VecDeque<(MsgId, LinkFrame)>,
    normal: VecDeque<(MsgId, LinkFrame)>,
    high: VecDeque<(MsgId, LinkFrame)>,
}

impl Queues {
    pub fn new() -> Self {
        Queues {
            low: VecDeque::new(),
            normal: VecDeque::new(),
            high: VecDeque::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.low.is_empty() && self.normal.is_empty() && self.high.is_empty()
    }
}

/// All packets are sent to the radio via the Outgoing queues by the regulator:
/// - easier to guarantee that we don't spam the mesh network
/// - can support priority
/// - can support coalescing actions which potentially reduce waste

#[derive(Debug)]
pub(crate) struct Outgoing {
    policy: OutgoingPolicy,
    queuesref: Arc<Mutex<Queues>>,
    notify: Option<mpsc::UnboundedSender<()>>,
}

impl Outgoing {
    pub(crate) fn new(link_config: &LinkConfig) -> Self {
        let queuesref = Arc::new(Mutex::new(Queues::new()));
        let policy = OutgoingPolicy::from_link_config(link_config);
        info!("OutgoingPolicy: {:?}", policy);
        Outgoing {
            policy,
            queuesref,
            notify: None,
        }
    }

    pub(crate) async fn qlen(&self) -> [usize; 3] {
        let queues = self.queuesref.lock().await;
        [queues.high.len(), queues.normal.len(), queues.low.len()]
    }

    pub(crate) async fn enqueue(
        &mut self,
        msgid: MsgId,
        frame: LinkFrame,
        options: LinkOptions,
    ) -> Action {
        let mut queues = self.queuesref.lock().await;
        let need_wakeup = queues.is_empty();
        let queue = match options.priority {
            Priority::Low => &mut queues.low,
            Priority::Normal => &mut queues.normal,
            Priority::High => &mut queues.high,
        };
        let outcome = if queue.len() > self.policy.outgoing_queue_max {
            Action::Limit
        } else {
            match options.action {
                Action::Drop => {
                    if !queue.iter().any(|(id, _)| *id == msgid) {
                        queue.push_back((msgid, frame)); // Drop if duplicate
                        Action::Queue
                    } else {
                        Action::Drop
                    }
                }
                Action::Queue => {
                    queue.push_back((msgid, frame)); // Always enqueue
                    Action::Queue
                }
                Action::Limit => {
                    error!("shouldn't see option.action set to Limit");
                    Action::Limit
                }
            }
        };
        if need_wakeup {
            if let Err(err) = self.notify.as_ref().unwrap().send(()) {
                error!("trouble sending notification to regulator: {:?}", err);
            }
        }
        outcome
    }

    pub(crate) async fn cancel(&mut self, msgid: MsgId, options: LinkOptions) {
        // When the link receives a message it should cancel any
        // outgoing copies of the message (another node beat us to
        // it) ...

        let mut queues = self.queuesref.lock().await;

        // Determine the queue to search based on priority
        let queue = match options.priority {
            Priority::Low => &mut queues.low,
            Priority::Normal => &mut queues.normal,
            Priority::High => &mut queues.high,
        };

        // Retain only those elements that do not match the given `msgid`
        let original_len = queue.len();
        queue.retain(|(id, _)| *id != msgid);

        let removed_count = original_len - queue.len();
        if removed_count > 0 {
            debug!("cancelled send of {} because already sent", msgid);
        }
    }

    pub(crate) fn start_regulator(&mut self, linkref: LinkRef) -> LinkResult<()> {
        let (notify, mut wake) = mpsc::unbounded_channel::<()>();
        self.notify = Some(notify);
        let queueref = self.queuesref.clone();
        let pause_msec = self.policy.tx_rate_limit_msec;
        task::spawn(async move {
            info!("outgoing regulator starting");
            loop {
                let mut queues = queueref.lock().await;
                match queues
                    .high
                    .pop_front()
                    .or_else(|| queues.normal.pop_front())
                    .or_else(|| queues.low.pop_front())
                {
                    Some((msgid, frame)) => {
                        let qlens = vec![queues.high.len(), queues.normal.len(), queues.low.len()];
                        drop(queues);

                        // Serialize the LinkFrame into bytes
                        let mut buffer = Vec::new();
                        if let Err(err) = frame.encode(&mut buffer) {
                            error!("trouble encoding link frame: {:?}", err);
                            continue;
                        }

                        // Escape a pathological sequence (see noshtastic#15)
                        let escaped_buffer = crate::escape94c3(&buffer);

                        // scope the link lock
                        {
                            let mut link = linkref.lock().await;

                            info!(
                                "qlens: {:?}: sending LinkFrame {}, sz: {}",
                                qlens,
                                msgid,
                                escaped_buffer.len()
                            );
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
                            match link
                                .stream_api
                                .send_mesh_packet(
                                    &mut router,
                                    escaped_buffer.into(),
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
                                Ok(mesh_packet_id) => debug!("mesh_packet_id: {}", mesh_packet_id),
                                Err(err) => error!("send_mesh_packet failed {:?}", err),
                            }
                        }

                        // IMPORTANT - it's important not to overload the mesh
                        // network.  Don't send packets back-to-back!
                        sleep(Duration::from_millis(pause_msec)).await;
                    }
                    None => {
                        drop(queues);
                        match wake.recv().await {
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }
            info!("outgoing regulator finished");
        });
        Ok(())
    }
}

pub(crate) struct LinkPacketRouter {
    my_id: NodeId,
}

impl PacketRouter<(), LinkError> for LinkPacketRouter {
    fn handle_packet_from_radio(&mut self, _packet: FromRadio) -> Result<(), LinkError> {
        Ok(())
    }

    fn handle_mesh_packet(&mut self, _packet: MeshPacket) -> Result<(), LinkError> {
        Ok(())
    }

    fn source_node_id(&self) -> NodeId {
        self.my_id
    }
}
