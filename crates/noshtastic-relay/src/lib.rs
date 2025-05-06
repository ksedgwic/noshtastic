// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

pub mod error;

pub use error::*;

pub use nostr_ndb::NdbDatabase;
use nostr_relay_builder::prelude::*;
use nostrdb::Ndb;

const RELAY_LOCAL_PORT: u16 = 8888;

/// Start a localhost relay asynchronously
pub async fn start_localhost_relay(ndb: Ndb) -> RelayResult<LocalRelay> {
    let builder = RelayBuilder::default()
        .port(RELAY_LOCAL_PORT)
        .database(NdbDatabase::from(ndb));
    let relay = LocalRelay::run(builder).await?;
    Ok(relay)
}
