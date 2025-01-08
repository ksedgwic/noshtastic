// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use nostrdb::Filter;
use nostrdb::Ndb;
use nostrdb::NoteKey;
use nostrdb::Transaction;
use prost::Message;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task;
use tokio::time::{self, sleep, Duration};

use noshtastic_link::{self, LinkMessage, LinkRef, MsgId};

use crate::{LruSet, Payload, Ping, Pong, RawNote, SyncError, SyncMessage, SyncResult};

#[derive(Debug)]
pub struct Sync {
    ndb: Ndb,
    _linkref: LinkRef,
    link_tx: mpsc::Sender<LinkMessage>,
    ping_duration: Option<Duration>, // None means no pinging
    max_notes: u32,
    recently_inserted: LruSet<MsgId>,
}
pub type SyncRef = Arc<std::sync::Mutex<Sync>>;

impl Sync {
    pub fn new(
        ndb: Ndb,
        _linkref: LinkRef,
        link_tx: mpsc::Sender<LinkMessage>,
        link_rx: mpsc::Receiver<LinkMessage>,
    ) -> SyncResult<SyncRef> {
        let syncref = Arc::new(Mutex::new(Sync {
            ndb: ndb.clone(),
            _linkref,
            link_tx,
            ping_duration: None,
            max_notes: 100,
            recently_inserted: LruSet::new(20),
        }));
        Self::start_message_handler(syncref.clone(), link_rx);
        Self::start_local_subscription(syncref.clone())?;
        Ok(syncref)
    }

    fn start_local_subscription(syncref: SyncRef) -> SyncResult<()> {
        let sync = syncref.lock().unwrap();
        let filter = Filter::new()
            .kinds([1])
            .limit(sync.max_notes as u64)
            .build();
        let ndbsubid = sync.ndb.subscribe(&[filter.clone()])?;

        let syncref_clone = syncref.clone();
        let ndb_clone = sync.ndb.clone();
        let max_notes = sync.max_notes;
        task::spawn(async move {
            debug!("local subscription starting");
            loop {
                debug!("waiting on local subscription");
                match ndb_clone.wait_for_notes(ndbsubid, max_notes).await {
                    Ok(notekeys) => {
                        info!("saw notekeys: {:?}", notekeys);
                        let sync = syncref_clone.lock().unwrap();
                        for notekey in notekeys {
                            if let Err(err) = sync.relay_notekey(notekey) {
                                error!("Error in relay_note: {:?}", err);
                                // keep going for now
                            }
                        }
                    }
                    Err(err) => {
                        error!("Error waiting for notes: {:?}", err);
                        // keep going for now
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
                let msgid: MsgId = note_json.as_bytes().into();
                if self.recently_inserted.contains(&msgid) {
                    // we don't need to automatically send messages
                    // that we just heard from the mesh
                    debug!("suppressing send of recently inserted {}", msgid);
                } else {
                    dbg!(&note_json);
                    self.send_raw_note(&note_json)?;
                }
            }
            Err(err) => error!("error in get_note_by_key: {:?}", err),
        }
        Ok(())
    }

    fn start_message_handler(syncref: SyncRef, mut receiver: mpsc::Receiver<LinkMessage>) {
        tokio::spawn(async move {
            info!("sync message handler starting");
            while let Some(message) = receiver.recv().await {
                match SyncMessage::decode(message.to_bytes()) {
                    Ok(decoded) => Self::handle_sync_message(syncref.clone(), decoded),
                    Err(e) => error!("Failed to decode message: {}", e),
                }
            }
            info!("sync message handler finished");
        });
    }

    fn handle_sync_message(syncref: SyncRef, message: SyncMessage) {
        let mut sync = syncref.lock().unwrap();
        match message.payload {
            Some(Payload::Ping(ping)) => {
                info!("received Ping id: {}", ping.id);
                Sync::after_delay(syncref.clone(), Duration::from_secs(1), move |sync| {
                    if let Err(err) = sync.send_pong(ping.id) {
                        error!("trouble sending pong: {:?}", err);
                    }
                })
            }
            Some(Payload::Pong(pong)) => {
                info!("received Pong id: {}", pong.id);
            }
            Some(Payload::RawNote(raw_note)) => {
                info!("received RawNote sz: {}", raw_note.data.len());
                sync.handle_raw_note(raw_note);
            }
            None => {
                warn!("received SyncMessage with no payload");
            }
        }
    }

    fn handle_raw_note(&mut self, raw_note: RawNote) {
        if let Ok(utf8_str) = std::str::from_utf8(&raw_note.data) {
            debug!("saw RawNote: {}", utf8_str);
            if let Err(err) = self.ndb.process_client_event(utf8_str) {
                error!("ndb process_client_event failed: {}: {:?}", &utf8_str, err);
            }
            let msgid: MsgId = utf8_str.as_bytes().into();
            self.recently_inserted.insert(msgid);
        } else {
            debug!("saw RawNote: [Invalid UTF-8 data: {:x?}]", raw_note.data);
        }
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

    fn queue_outgoing_message(&self, payload: Option<Payload>) -> SyncResult<()> {
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
            runtime.block_on(async { self.link_tx.send(buffer.into()).await })
        })
        .map_err(|err| {
            SyncError::internal_error(format!(
                "queue_outgoing_message: mspc send error: {:?}",
                err
            ))
        })?;
        Ok(())
    }

    fn send_raw_note(&self, eventbuf: &str) -> SyncResult<()> {
        info!("sending RawNote sz: {}", eventbuf.len());
        let raw_note = Payload::RawNote(RawNote {
            data: eventbuf.as_bytes().to_vec(),
        });
        self.queue_outgoing_message(Some(raw_note))
    }

    fn send_ping(&self, ping_id: u32) -> SyncResult<()> {
        info!("sending Ping id: {}", ping_id);
        self.queue_outgoing_message(Some(Payload::Ping(Ping { id: ping_id })))
    }

    fn send_pong(&self, pong_id: u32) -> SyncResult<()> {
        info!("sending Pong id: {}", pong_id);
        self.queue_outgoing_message(Some(Payload::Pong(Pong { id: pong_id })))
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
