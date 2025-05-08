// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

pub mod error;

pub use error::*;

use log::*;
use nostr::Event;
use nostr::JsonUtil;
pub use nostr_ndb::NdbDatabase;
use nostr_relay_builder::prelude::*;
use nostrdb::Ndb;
use tokio::sync::mpsc::UnboundedReceiver;

const RELAY_LOCAL_PORT: u16 = 8888;

/// Start a localhost relay asynchronously
pub async fn start_localhost_relay(
    ndb: Ndb,
    mut incoming: UnboundedReceiver<String>,
) -> RelayResult<LocalRelay> {
    let builder = RelayBuilder::default()
        .port(RELAY_LOCAL_PORT)
        .database(NdbDatabase::from(ndb));
    let relay = LocalRelay::run(builder).await?;

    // forward any mesh-received events into the relay’s notify_event
    let notify_relay = relay.clone();
    tokio::spawn(async move {
        while let Some(event_json) = incoming.recv().await {
            info!("sending incoming event to relay: {}", event_json);
            match Event::from_json(event_json) {
                Ok(evt) => {
                    notify_relay.notify_event(evt);
                }
                Err(err) => error!("trouble in notify_relay.notify_event: {:?}", err),
            }
        }
    });

    Ok(relay)
}
