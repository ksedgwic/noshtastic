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

use crate::{Action, LinkError, LinkFrame, LinkOptions, LinkRef, LinkResult, MsgId, Priority};

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
    queuesref: Arc<Mutex<Queues>>,
    notify: Option<mpsc::Sender<()>>,
}

impl Outgoing {
    pub(crate) fn new() -> Self {
        let queuesref = Arc::new(Mutex::new(Queues::new()));
        Outgoing {
            queuesref,
            notify: None,
        }
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
        let outcome = match options.action {
            Action::Drop => {
                if !queue.iter().any(|(id, _)| *id == msgid) {
                    queue.push_back((msgid, frame)); // Enqueue only if no match
                    Action::Queue
                } else {
                    Action::Drop
                }
            }
            Action::Queue => {
                queue.push_back((msgid, frame)); // Always enqueue
                Action::Queue
            }
        };
        if need_wakeup {
            if let Err(err) = self.notify.as_ref().unwrap().send(()).await {
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
        let (notify, mut wake) = mpsc::channel::<()>(100);
        self.notify = Some(notify);
        let queueref = self.queuesref.clone();
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
                        drop(queues);

                        // Serialize the LinkFrame into bytes
                        let mut buffer = Vec::new();
                        if let Err(err) = frame.encode(&mut buffer) {
                            error!("trouble encoding link frame: {:?}", err);
                            continue;
                        }

                        // scope the link lock
                        {
                            let mut link = linkref.lock().await;

                            debug!("sending LinkFrame {}, sz: {}", msgid, buffer.len());
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
                        }

                        // IMPORTANT - it's important not to overload the mesh
                        // network.  Don't send packets back-to-back!
                        sleep(Duration::from_secs(5)).await;
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
