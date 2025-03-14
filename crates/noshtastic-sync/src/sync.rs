// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use futures::StreamExt;
use log::*;
use nostrdb::{Filter, Ndb, NoteKey, Transaction};
use prost::Message;
use rand::{seq::SliceRandom, thread_rng};
use std::{
    fmt,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{mpsc, Notify},
    task,
    time::{self, sleep, Duration},
};

use noshtastic_link::{
    self, Action, LinkMessage, LinkOptions, LinkOptionsBuilder, MsgId, Priority,
};

use crate::{
    negentropy::NegentropyState, EncNote, LruSet, NegentropyMessage, Payload, Ping, Pong, RawNote,
    SyncError, SyncMessage, SyncResult,
};

pub struct Sync {
    ndb: Ndb,
    link_tx: mpsc::Sender<LinkMessage>,
    ping_duration: Option<Duration>, // None means no pinging
    max_notes: u32,
    recently_inserted: LruSet<MsgId>,
    negentropy: NegentropyState,
}
pub type SyncRef = Arc<std::sync::Mutex<Sync>>;

impl Sync {
    pub fn new(
        ndb: Ndb,
        link_tx: mpsc::Sender<LinkMessage>,
        link_rx: mpsc::Receiver<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) -> SyncResult<SyncRef> {
        let syncref = Arc::new(Mutex::new(Sync {
            ndb: ndb.clone(),
            link_tx,
            ping_duration: None,
            max_notes: 100,
            recently_inserted: LruSet::new(20),
            negentropy: NegentropyState::new(ndb.clone()),
        }));
        Self::start_message_handler(syncref.clone(), link_rx, stop_signal.clone());
        Self::start_local_subscription(syncref.clone(), stop_signal.clone())?;
        Self::start_negentropy_sync(syncref.clone(), stop_signal.clone())?;
        Ok(syncref)
    }

    fn start_negentropy_sync(syncref: SyncRef, stop_signal: Arc<Notify>) -> SyncResult<()> {
        task::spawn(async move {
            debug!("negentropy sync starting");
            sleep(Duration::from_secs(5)).await; // give ndb a chance to get setup
            let mut interval = time::interval(Duration::from_secs(5 * 60));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut sync = syncref.lock().unwrap();
                        match sync.negentropy.initiate() {
                            Err(err) => error!("trouble in negentropy initiate: {:?}", err),
                            Ok(negbytes) =>
                                if let Err(err) = sync.send_negentropy_message(&negbytes, true) {
                                    error!("trouble queueing negentropy message: {:?}", err);
                                },
                        }
                    }
                    _ = stop_signal.notified() => {
                        break;
                    }
                }
            }
            debug!("negentropy sync stopping");
        });

        Ok(())
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
                        // TEMPORARY - for now don't immediately send new notes
                        if false {
                            let sync = syncref_clone.lock().unwrap();
                            for notekey in notekeys {
                                if let Err(err) = sync.relay_notekey(notekey) {
                                    error!("Error in relay_note: {:?}", err);
                                    // keep going for now
                                }
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
                    dbg!(&note_json);
                    self.send_encoded_note(MsgId::from_nostr_msgid(note.id()), &note_json)?;
                }
            }
            Err(err) => error!("error in get_note_by_key: {:?}", err),
        }
        Ok(())
    }

    fn start_message_handler(
        syncref: SyncRef,
        mut receiver: mpsc::Receiver<LinkMessage>,
        stop_signal: Arc<Notify>,
    ) {
        tokio::spawn(async move {
            info!("sync message handler starting");
            loop {
                tokio::select! {
                    Some(msg) = receiver.recv() => {
                        match SyncMessage::decode(&msg.data[..]) {
                            Ok(decoded) => {
                                Self::handle_sync_message(syncref.clone(), msg.msgid, decoded)
                            },
                            Err(e) => error!("Failed to decode message: {}", e),
                        }
                    }
                    _ = stop_signal.notified() => {
                        break;
                    }

                }
                info!("sync message handler finished");
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
                sync.handle_raw_note(msgid, raw_note);
            }
            Some(Payload::EncNote(enc_note)) => {
                info!("received EncNote {}", msgid);
                sync.handle_enc_note(msgid, enc_note);
            }
            Some(Payload::Negentropy(negmsg)) => {
                info!("received NegentropyMessage sz: {}", negmsg.data.len());
                let mut have_ids = vec![];
                let mut need_ids = vec![];
                match sync
                    .negentropy
                    .reconcile(&negmsg.data, &mut have_ids, &mut need_ids)
                {
                    Err(err) => error!("trouble reconciling negentropy message: {:?}", err),
                    Ok(None) => info!("synchronized with remote"),
                    Ok(Some(nextmsg)) => {
                        if let Err(err) = sync.send_negentropy_message(&nextmsg, false) {
                            error!("trouble queueing next negentropy message: {:?}", err);
                        }
                    }
                }
                debug!("have: {:?}", DebugVecId(have_ids.clone()));
                debug!("need: {:?}", DebugVecId(need_ids.clone()));
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
            if let Err(err) = self.ndb.process_client_event(utf8_str) {
                error!(
                    "ndb process_client_event (raw) failed: {}: {:?}",
                    &utf8_str, err
                );
            }
            self.recently_inserted.insert(msgid);
        } else {
            debug!("saw RawNote: [Invalid UTF-8 data: {:x?}]", raw_note.data);
        }
    }

    fn handle_enc_note(&mut self, msgid: MsgId, enc_note: EncNote) {
        let utf8_str = enc_note.to_string();
        debug!("saw EncNote {}: {}", msgid, utf8_str);
        if let Err(err) = self.ndb.process_client_event(&utf8_str) {
            error!(
                "ndb process_client_event (enc) failed: {}: {:?}",
                &utf8_str, err
            );
        }
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
        options: LinkOptions,
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

        // queue the outgoing message, this shouldn't block
        task::block_in_place(|| {
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(async {
                self.link_tx
                    .send(LinkMessage {
                        msgid,
                        data: buffer,
                        options,
                    })
                    .await
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

    fn send_negentropy_message(&self, data: &[u8], is_initial: bool) -> SyncResult<()> {
        info!(
            "queueing NegentropyMessage, is_initial: {}, sz: {}",
            is_initial,
            data.len()
        );
        let negmsg = Payload::Negentropy(NegentropyMessage {
            data: data.to_vec(),
        });
        let priority = if is_initial {
            // defer initial messages if we are busy
            Priority::Low
        } else {
            // send response messages promptly
            Priority::High
        };
        let msgid = MsgId::from(data);
        self.queue_outgoing_message(
            msgid,
            Some(negmsg),
            LinkOptionsBuilder::new().priority(priority).build(),
        )
    }

    // fn send_raw_note(&self, msgid: MsgId, note_json: &str) -> SyncResult<()> {
    //     info!("queueing RawNote {} sz: {}", msgid, note_json.len());
    //     let raw_note = Payload::RawNote(RawNote {
    //         data: note_json.as_bytes().to_vec(),
    //     });
    //     self.queue_outgoing_message(
    //         msgid, Some(raw_note), LinkOptionsBuilder::new().action(Action::Drop).build()
    //     )
    // }

    fn send_encoded_note(&self, msgid: MsgId, note_json: &str) -> SyncResult<()> {
        info!("queueing EncNote {} sz: {}", msgid, note_json.len());
        let enc_note = Payload::EncNote(EncNote::try_from(note_json)?);
        self.queue_outgoing_message(
            msgid,
            Some(enc_note),
            LinkOptionsBuilder::new().action(Action::Drop).build(),
        )
    }

    fn send_ping(&self, ping_id: u32) -> SyncResult<()> {
        info!("queueing Ping id: {}", ping_id);
        self.queue_outgoing_message(
            MsgId::new(Self::ID_PING, None),
            Some(Payload::Ping(Ping { id: ping_id })),
            LinkOptionsBuilder::new().build(),
        )
    }

    fn send_pong(&self, pong_id: u32) -> SyncResult<()> {
        info!("queueing Pong id: {}", pong_id);
        self.queue_outgoing_message(
            MsgId::new(Self::ID_PONG, None),
            Some(Payload::Pong(Pong { id: pong_id })),
            LinkOptionsBuilder::new().build(),
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
