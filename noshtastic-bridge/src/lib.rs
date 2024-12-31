// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use enostr::ewebsock::{WsEvent, WsMessage};
use enostr::RelayPool;
use log::*;
use nostrdb::Filter;
use nostrdb::Ndb;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub mod error;
pub use error::*;

pub struct Bridge {
    ndb: Ndb,
    opt_relay_url: Option<String>,
    #[allow(dead_code)] // FIXME - remove this
    opt_filter_json: Option<String>,
    poolref: Arc<Mutex<RelayPool>>,
}

impl Bridge {
    pub fn new(
        ndb: Ndb,
        opt_relay_url: &Option<String>,
        opt_filter_json: &Option<String>,
    ) -> BridgeResult<Self> {
        let poolref = Arc::new(Mutex::new(RelayPool::new()));
        Ok(Bridge {
            ndb,
            opt_relay_url: opt_relay_url.clone(),
            opt_filter_json: opt_filter_json.clone(),
            poolref,
        })
    }

    pub fn start(&mut self) -> BridgeResult<()> {
        if self.opt_relay_url.is_none() || self.opt_filter_json.is_none() {
            info!("bridge not configured, skipping");
        }
        let relay_url = self.opt_relay_url.as_ref().unwrap().clone();
        let filter_json = &self.opt_filter_json.as_ref().unwrap().clone();

        // create a pool event handler which inserts the events into ndb
        let ndb_clone = self.ndb.clone();
        let poolref_clone = self.poolref.clone();
        let handle_pool_event = move || {
            let mut pool = poolref_clone.lock().unwrap();
            while let Some(event) = pool.try_recv() {
                match event.event {
                    WsEvent::Message(WsMessage::Text(text)) => {
                        debug!("saw text msg: {}", text);
                        if let Err(err) = ndb_clone.process_event(&text) {
                            error!("error processing event {}: {:?}", text, err);
                        }
                    }
                    WsEvent::Message(msg) => {
                        debug!("saw other message type: {:?}", msg);
                    }
                    other => {
                        debug!("saw other event: {:?}", other);
                    }
                }
            }
        };

        let mut pool = self.poolref.lock().unwrap();
        info!("bridge to {} starting", relay_url);
        pool.add_url(relay_url.clone(), handle_pool_event)?;
        let subid = Uuid::new_v4().to_string();
        let filter = Filter::from_json(filter_json)?;
        pool.subscribe(subid.clone(), vec![filter.clone()]);
        Ok(())
    }

    pub fn stop(&mut self) -> BridgeResult<()> {
        if self.opt_relay_url.is_some() && self.opt_filter_json.is_some() {
            info!(
                "bridge to {} stopping",
                self.opt_relay_url.as_ref().unwrap()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    use env_logger;
    use hex;
    use nostrdb::Filter;
    use once_cell::sync::Lazy;

    static _INIT: Lazy<()> = Lazy::new(|| {
        env_logger::builder()
            .is_test(true)
            .filter_level(log::LevelFilter::Debug)
            .init();
    });

    #[test]
    fn test_ndb_filter_json() {
        let author_hexes = vec!["379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe"];
        let authors: Vec<[u8; 32]> = author_hexes
            .into_iter()
            .map(|hex| {
                let decoded = hex::decode(hex).expect("valid hex string");
                decoded.try_into().expect("expected 32-byte array")
            })
            .collect();
        let filter = Filter::new().authors(authors.iter()).kinds([1]).build();
        dbg!(authors);
        assert_eq!(
            filter.json().unwrap(),
            r#"{"authors":["379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe"],"kinds":[1]}"#
        );
    }
}
