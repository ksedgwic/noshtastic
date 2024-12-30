use log::*;
use nostrdb::Ndb;
use prost::Message;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tokio::task;
use tokio::time::{self, sleep, Duration};

use noshtastic_link::{self, LinkError, LinkMessage, LinkRef};

pub mod proto {
    include!("../protos/noshtastic_sync.rs");
}
pub use proto::sync_message::Payload;
pub use proto::Ping;
pub use proto::Pong;
pub use proto::SyncMessage;

pub mod error;
pub use error::*;

use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Sync {
    _ndb: Ndb,
    linkref: LinkRef,
    ping_duration: Option<Duration>, // None means no pinging
}
pub type SyncRef = Arc<std::sync::Mutex<Sync>>;

impl Sync {
    pub fn new(
        _ndb: Ndb,
        linkref: LinkRef,
        receiver: mpsc::Receiver<LinkMessage>,
    ) -> SyncResult<SyncRef> {
        let syncref = Arc::new(Mutex::new(Sync {
            _ndb,
            linkref,
            ping_duration: None,
        }));
        Self::start_message_handler(receiver, syncref.clone());
        Ok(syncref)
    }

    fn start_message_handler(mut receiver: mpsc::Receiver<LinkMessage>, syncref: SyncRef) {
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
        let _sync = syncref.lock().unwrap();
        match message.payload {
            Some(Payload::Ping(ping)) => {
                info!("received ping id: {}", ping.id);
                Sync::after_delay(syncref.clone(), Duration::from_secs(1), move |sync| {
                    if let Err(err) = sync.send_pong(ping.id) {
                        error!("trouble sending pong: {:?}", err);
                    }
                })
            }
            Some(Payload::Pong(pong)) => {
                info!("received pong id: {}", pong.id);
            }
            None => {
                warn!("received SyncMessage with no payload");
            }
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

    fn send_message(&self, payload: Option<Payload>) -> SyncResult<()> {
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

        // Send the serialized message
        task::block_in_place(|| {
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(async {
                self.linkref
                    .lock()
                    .await
                    .queue_message(buffer.into())
                    .await?;
                Ok::<(), LinkError>(()) // ensure compatibility with `LinkError`
            })
        })
        .map_err(SyncError::from)?;

        Ok(())
    }

    fn send_ping(&self, ping_id: u32) -> SyncResult<()> {
        info!("sending ping id: {}", ping_id);
        self.send_message(Some(Payload::Ping(Ping { id: ping_id })))
    }

    fn send_pong(&self, pong_id: u32) -> SyncResult<()> {
        info!("sending pong id: {}", pong_id);
        self.send_message(Some(Payload::Pong(Pong { id: pong_id })))
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
