// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use futures::StreamExt;
use log::*;
use nostrdb::{Filter, IngestMetadata, Ndb, NoteKey, Transaction};
use prost::Message;
use rand::{seq::SliceRandom, thread_rng};
use std::{
    fmt,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::{
    sync::{mpsc, Notify},
    task,
    time::{self, sleep, Duration, Instant},
};

use noshtastic_link::{
    self, LinkConfig, LinkInMessage, LinkInfo, LinkOutMessage, LinkOutOptions,
    LinkOutOptionsBuilder, LinkOutPayload, MsgId, Priority,
};

use crate::{
    negentropy::NegentropyState, EncNote, LruSet, NegentropyMessage, Payload, Ping, Pong, RawNote,
    SyncError, SyncMessage, SyncResult,
};

#[derive(Debug)]
struct SyncPolicy {
    outgoing_refill_thresh: usize,
    outgoing_pause_thresh: usize,
    note_idle_secs: u64,
    reconcile_idle_secs: u64,
}

impl SyncPolicy {
    fn from_link_config(link_cfg: &LinkConfig) -> Self {
        // baseline data rate for “defaults” = 3.52 kbps (MEDIUM_FAST)
        const BASE_DATA_KBPS: f32 = 3.52;
        const OUTGOING_REFILL_THRESH: f32 = 10.0;
        const OUTGOING_PAUSE_THRESH: f32 = 50.0;
        const NOTE_IDLE_SECS: f32 = 60.0;
        const RECONCILE_IDLE_SECS: f32 = 120.0;

        let queue_scale = link_cfg.data_kbps / BASE_DATA_KBPS;
        let rate_scale = BASE_DATA_KBPS / link_cfg.data_kbps;

        // scale to match other data rates
        let outgoing_refill_thresh =
            (OUTGOING_REFILL_THRESH * queue_scale).max(1.0).round() as usize;
        let outgoing_pause_thresh = (OUTGOING_PAUSE_THRESH * queue_scale)
            .max((outgoing_refill_thresh + 1) as f32)
            .round() as usize;
        let note_idle_secs = (NOTE_IDLE_SECS * rate_scale).max(1.0).round() as u64;
        let reconcile_idle_secs = (RECONCILE_IDLE_SECS * rate_scale).max(1.0).round() as u64;

        SyncPolicy {
            outgoing_refill_thresh,
            outgoing_pause_thresh,
            note_idle_secs,
            reconcile_idle_secs,
        }
    }
}

pub struct Sync {
    policy: SyncPolicy,
    _stop_signal: Arc<Notify>,
    ndb: Ndb,
    link_tx: mpsc::UnboundedSender<LinkOutMessage>,
    incoming_tx: mpsc::UnboundedSender<String>,
    ping_duration: Option<Duration>, // None means no pinging
    max_notes: u32,
    recently_inserted: LruSet<MsgId>,
    negentropy: NegentropyState,
    is_link_ready: bool,
    last_sync_recv: Instant,
    last_note_recv: Instant,
    pause_sync: bool, // true when we have too many outgoing messages
}
pub type SyncRef = Arc<std::sync::Mutex<Sync>>;

impl Sync {
    pub fn new(
        link_config: &LinkConfig,
        ndb: Ndb,
        link_tx: mpsc::UnboundedSender<LinkOutMessage>,
        link_rx: mpsc::UnboundedReceiver<LinkInMessage>,
        incoming_tx: UnboundedSender<String>,
        stop_signal: Arc<Notify>,
    ) -> SyncResult<SyncRef> {
        let long_ago = Instant::now() - Duration::from_secs(u64::MAX / 2);
        let policy = SyncPolicy::from_link_config(link_config);
        info!("SyncPolicy: {:?}", policy);
        let syncref = Arc::new(Mutex::new(Sync {
            policy,
            _stop_signal: stop_signal.clone(),
            ndb: ndb.clone(),
            link_tx,
            incoming_tx,
            ping_duration: None,
            max_notes: 100,
            recently_inserted: LruSet::new(20),
            negentropy: NegentropyState::new(ndb.clone()),
            is_link_ready: false,
            last_sync_recv: long_ago,
            last_note_recv: long_ago,
            pause_sync: false,
        }));
        Self::start_message_handler(syncref.clone(), link_rx, stop_signal.clone());
        Self::start_local_subscription(syncref.clone(), stop_signal.clone())?;
        Ok(syncref)
    }

    // called when the link is ready
    fn link_ready(syncref: SyncRef) {
        let mut sync = syncref.lock().unwrap();
        sync.is_link_ready = true;
    }

    // called when a LinkInfo packet is received
    fn link_info(syncref: SyncRef, info: LinkInfo) {
        let mut sync = syncref.lock().unwrap();
        info!(
            "saw LinkInMessage::Info: {:?}, last_sync: {} last_note: {}",
            &info,
            sync.last_sync_recv.elapsed().as_secs(),
            sync.last_note_recv.elapsed().as_secs(),
        );

        // There are two levels of sync participation:
        // sync initiation     - initiate a sync protocol exchange
        // sync reconciliation - respond to sync protocol from other notes

        let qlen = info.qlen[0] + info.qlen[1];

        let should_pause = qlen >= sync.policy.outgoing_pause_thresh;
        if should_pause != sync.pause_sync {
            sync.pause_sync = should_pause;
            if should_pause {
                info!(
                    "outgoing queues full {}, pausing sync reconciliation until {}",
                    qlen, sync.policy.outgoing_pause_thresh
                );
                return;
            } else {
                info!(
                    "outgoing queues not full {}, resuming sync reconciliation until {}",
                    qlen, sync.policy.outgoing_pause_thresh
                );
                // drop through here to check for other things
            }
        }

        // This routine should decide when to initiate negentropy messages

        if !sync.is_link_ready {
            info!("link is not ready, defer sync initiation");
            return;
        }

        if sync.last_note_recv.elapsed().as_secs() < sync.policy.note_idle_secs {
            info!(
                "received notes {} secs ago, defer sync initiation until {}",
                sync.last_note_recv.elapsed().as_secs(),
                sync.policy.note_idle_secs
            );
            return;
        }

        if sync.last_sync_recv.elapsed().as_secs() < sync.policy.reconcile_idle_secs {
            info!(
                "reconciled {} secs ago, defer sync initiation until {}",
                sync.last_sync_recv.elapsed().as_secs(),
                sync.policy.reconcile_idle_secs
            );
            return;
        }

        if qlen > sync.policy.outgoing_refill_thresh {
            info!(
                "outgoing queues full {}, defer sync initiation until {}",
                qlen, sync.policy.outgoing_refill_thresh
            );
            return;
        }

        // if we make it here initiate a negentropy exchange
        match sync.negentropy.initiate() {
            Err(err) => error!("trouble in sync initiation: {:?}", err),
            Ok(negbytes) => {
                if let Err(err) = sync.send_negentropy_message(&negbytes, 0) {
                    error!("trouble queueing negentropy message: {:?}", err);
                }
            }
        }
    }

    fn start_local_subscription(syncref: SyncRef, _stop_signal: Arc<Notify>) -> SyncResult<()> {
        let sync = syncref.lock().unwrap();
        let syncref_clone = syncref.clone();
        let max_notes = sync.max_notes;
        let filter = Filter::new().kinds([1]).limit(max_notes as u64).build();
        let ndbsubid = sync.ndb.subscribe(&[filter.clone()]).unwrap();
        let mut sub = ndbsubid.stream(&sync.ndb).notes_per_await(1);
        task::spawn(async move {
            debug!("local subscription starting");
            loop {
                debug!("waiting on local subscription");
                match sub.next().await {
                    Some(notekeys) => {
                        info!("saw notekeys: {:?}", notekeys);
                        let sync = syncref_clone.lock().unwrap();
                        for notekey in notekeys {
                            if let Err(err) = sync.relay_notekey(notekey) {
                                error!("Error in relay_note: {:?}", err);
                                // keep going for now
                            }
                        }
                    }
                    None => {
                        // we're done
                        break;
                    }
                }
            }
            #[allow(unreachable_code)]
            {
                debug!("local subscription finished");
            }
        });

        Ok(())
    }

    fn relay_notekey(&self, notekey: NoteKey) -> SyncResult<()> {
        let txn = Transaction::new(&self.ndb).expect("txn");
        match self.ndb.get_note_by_key(&txn, notekey) {
            Ok(note) => {
                let note_json = note.json()?;
                let msgid = MsgId::from_nostr_msgid(note.id());
                if self.recently_inserted.contains(&msgid) {
                    // we don't want send messages that we just heard
                    // from the mesh
                    debug!("suppressing send of recently inserted {}", msgid);
                } else {
                    self.send_encoded_note(MsgId::from_nostr_msgid(note.id()), &note_json)?;
                }
            }
            Err(err) => error!("error in get_note_by_key: {:?}", err),
        }
        Ok(())
    }

    fn start_message_handler(
        syncref: SyncRef,
        mut receiver: mpsc::UnboundedReceiver<LinkInMessage>,
        stop_signal: Arc<Notify>,
    ) {
        tokio::spawn(async move {
            debug!("sync message handler starting");
            loop {
                tokio::select! {
                    Some(msg) = receiver.recv() => {
                        match msg {
                            LinkInMessage::Ready => {
                                Self::link_ready(syncref.clone());
                            },
                            LinkInMessage::Info(info) => {
                                Self::link_info(syncref.clone(), info);
                            },
                            LinkInMessage::Payload(payload) => {
                                match SyncMessage::decode(&payload.data[..]) {
                                    Ok(decoded) => {
                                        Self::handle_sync_message(
                                            syncref.clone(), payload.msgid, decoded)
                                    },
                                    Err(e) => error!("Failed to decode message: {}", e),
                                }
                            },
                        }
                    },
                    _ = stop_signal.notified() => {
                        break;
                    },
                }
                debug!("sync message handler finished");
            }
        });
    }

    fn handle_sync_message(syncref: SyncRef, msgid: MsgId, message: SyncMessage) {
        let mut sync = syncref.lock().unwrap();
        match message.payload {
            Some(Payload::Ping(ping)) => {
                info!("received Ping id: {}", ping.id);
                Sync::after_delay(syncref.clone(), Duration::from_secs(1), move |sync| {
                    if let Err(err) = sync.send_pong(ping.id) {
                        error!("trouble queueing pong: {:?}", err);
                    }
                })
            }
            Some(Payload::Pong(pong)) => {
                info!("received Pong id: {}", pong.id);
            }
            Some(Payload::RawNote(raw_note)) => {
                info!("received RawNote {} sz: {}", msgid, raw_note.data.len());
                sync.last_note_recv = Instant::now();
                sync.handle_raw_note(msgid, raw_note);
            }
            Some(Payload::EncNote(enc_note)) => {
                info!("received EncNote {}", msgid);
                sync.last_note_recv = Instant::now();
                sync.handle_enc_note(msgid, enc_note);
            }
            Some(Payload::Negentropy(negmsg)) => {
                info!(
                    "received NegentropyMessage: level: {}, sz: {}",
                    negmsg.level,
                    negmsg.data.len()
                );
                sync.last_sync_recv = Instant::now();
                if sync.pause_sync {
                    info!("outgoing queues full, sync reconcilation paused");
                    return;
                }
                let mut have_ids = vec![];
                let mut need_ids = vec![];
                match sync
                    .negentropy
                    .reconcile(&negmsg.data, &mut have_ids, &mut need_ids)
                {
                    Err(err) => error!("trouble reconciling negentropy message: {:?}", err),
                    Ok(None) => info!("SYNCHRONIZED WITH REMOTE"),
                    Ok(Some(nextmsg)) => {
                        if let Err(err) = sync.send_negentropy_message(&nextmsg, negmsg.level + 1) {
                            error!("trouble queueing next negentropy message: {:?}", err);
                        }
                    }
                }
                info!("have: {:?}", DebugVecId(have_ids.clone()));
                info!("need: {:?}", DebugVecId(need_ids.clone()));
                if let Err(err) = sync.send_needed_notes(have_ids) {
                    error!("send needed notes failed: {:?}", err);
                }
            }
            None => {
                warn!("received SyncMessage with no payload");
            }
        }
    }

    fn handle_raw_note(&mut self, msgid: MsgId, raw_note: RawNote) {
        if let Ok(utf8_str) = std::str::from_utf8(&raw_note.data) {
            debug!("saw RawNote {}: {}", msgid, utf8_str);

            // first, store it in the db
            if let Err(err) = self
                .ndb
                .process_event_with(utf8_str, IngestMetadata::new().client(true))
            {
                error!(
                    "ndb process_client_event (raw) failed: {}: {:?}",
                    &utf8_str, err
                );
            }

            // then send it directly to the relay
            self.incoming_tx.send(utf8_str.to_string()).ok();

            self.recently_inserted.insert(msgid);
        } else {
            warn!("saw RawNote: [Invalid UTF-8 data: {:x?}]", raw_note.data);
        }
    }

    fn handle_enc_note(&mut self, msgid: MsgId, enc_note: EncNote) {
        let utf8_str = enc_note.to_string();
        debug!("saw EncNote {}: {}", msgid, utf8_str);

        // first, store it in the db
        if let Err(err) = self
            .ndb
            .process_event_with(&utf8_str, IngestMetadata::new().client(true))
        {
            error!(
                "ndb process_client_event (enc) failed: {}: {:?}",
                &utf8_str, err
            );
        }

        // then send it directly to the relay
        self.incoming_tx.send(utf8_str).ok();

        self.recently_inserted.insert(msgid);
    }

    pub fn after_delay<F, T>(syncref: Arc<Mutex<T>>, delay: Duration, task: F)
    where
        F: FnOnce(&mut T) + Send + 'static,
        T: Send + 'static,
    {
        tokio::spawn(async move {
            sleep(delay).await; // Wait for the specified duration
            let mut sync = syncref.lock().unwrap();
            task(&mut *sync); // Execute the closure
        });
    }

    fn queue_outgoing_message(
        &self,
        msgid: MsgId,
        payload: Option<Payload>,
        options: LinkOutOptions,
    ) -> SyncResult<()> {
        // Create a SyncMessage
        let message = SyncMessage {
            version: 1, // Protocol version
            payload,
        };

        // Serialize the message to bytes
        let mut buffer = Vec::new();
        message.encode(&mut buffer).map_err(|err| {
            SyncError::internal_error(format!("sync: failed to encode message: {:?}", err))
        })?;

        // queue the outgoing message, this won't really block
        task::block_in_place(|| {
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(async {
                self.link_tx.send(LinkOutMessage::Payload(LinkOutPayload {
                    msgid,
                    data: buffer,
                    options,
                }))
            })
        })
        .map_err(|err| {
            SyncError::internal_error(format!(
                "queue_outgoing_message: mspc send error: {:?}",
                err
            ))
        })?;
        Ok(())
    }

    fn send_needed_notes(&self, mut needed: Vec<Vec<u8>>) -> SyncResult<()> {
        // Shuffle the notes do reduce duplication w/ other nodes
        // responding to the same needs ...
        needed.shuffle(&mut thread_rng());

        let txn = Transaction::new(&self.ndb)?;
        for id in needed.iter() {
            if id.len() == 32 {
                let id_array: &[u8; 32] = id.as_slice().try_into().unwrap();
                match self.ndb.get_note_by_id(&txn, id_array) {
                    Err(err) => error!("trouble in get_note_by_id: {:?}", err),
                    Ok(note) => {
                        if let Ok(note_json) = note.json() {
                            if let Err(err) = self
                                .send_encoded_note(MsgId::from_nostr_msgid(note.id()), &note_json)
                            {
                                error!("trouble queueing needed raw note: {:?}", err);
                                // keep going
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // well-known message ids
    const ID_PING: u64 = 1;
    const ID_PONG: u64 = 2;

    fn send_negentropy_message(&self, data: &[u8], level: u32) -> SyncResult<()> {
        let negmsg = Payload::Negentropy(NegentropyMessage {
            data: data.to_vec(),
            level,
        });
        let msgid = MsgId::from(data);
        info!(
            "queueing NegentropyMessage {}: level: {}, sz: {}",
            msgid,
            level,
            data.len()
        );
        self.queue_outgoing_message(
            msgid,
            Some(negmsg),
            LinkOutOptionsBuilder::new()
                .priority(Priority::High)
                .build(),
        )
    }

    fn send_encoded_note(&self, msgid: MsgId, note_json: &str) -> SyncResult<()> {
        info!("queueing EncNote {} sz: {}", msgid, note_json.len());
        let enc_note = Payload::EncNote(EncNote::try_from(note_json)?);
        self.queue_outgoing_message(msgid, Some(enc_note), LinkOutOptionsBuilder::new().build())
    }

    fn send_ping(&self, ping_id: u32) -> SyncResult<()> {
        info!("queueing Ping id: {}", ping_id);
        self.queue_outgoing_message(
            MsgId::new(Self::ID_PING, None),
            Some(Payload::Ping(Ping { id: ping_id })),
            LinkOutOptionsBuilder::new().build(),
        )
    }

    fn send_pong(&self, pong_id: u32) -> SyncResult<()> {
        info!("queueing Pong id: {}", pong_id);
        self.queue_outgoing_message(
            MsgId::new(Self::ID_PONG, None),
            Some(Payload::Pong(Pong { id: pong_id })),
            LinkOutOptionsBuilder::new().build(),
        )
    }

    pub fn start_pinging(syncref: SyncRef, duration: Duration) -> SyncResult<()> {
        let mut sync = syncref.lock().unwrap();
        match sync.ping_duration {
            Some(_) => return Err(SyncError::operation_not_allowed("pinger already running")),
            None => sync.ping_duration = Some(duration),
        }

        let syncref_clone = syncref.clone();
        let ping_duration = sync.ping_duration;
        tokio::spawn(async move {
            info!("pinger starting started with interval {:?}", duration);
            let mut interval = time::interval(duration);
            let mut ping_id = 0;

            while ping_duration.is_some() {
                interval.tick().await;
                let sync = syncref_clone.lock().unwrap();
                sync.send_ping(ping_id).ok();
                ping_id += 1;
            }
            info!("pinger stopping");
        });

        Ok(())
    }

    /// Stop the pinger by setting `ping_interval` to `None`
    pub fn stop_pinging(&mut self) -> SyncResult<()> {
        match self.ping_duration {
            None => return Err(SyncError::operation_not_allowed("pinger not running")),
            Some(_) => self.ping_duration = None,
        }
        debug!("stopping pinger");
        Ok(())
    }
}

pub struct DebugVecId(Vec<Vec<u8>>);
impl fmt::Debug for DebugVecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex_strings: Vec<String> = self.0.iter().map(hex::encode).collect();
        write!(f, "[{}]", hex_strings.join(", "))
    }
}
