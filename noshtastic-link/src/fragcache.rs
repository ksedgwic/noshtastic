// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{proto::LinkMissing, LinkFrag, LinkMsg, PayloadId};

#[allow(dead_code)] // FIXME - remove this asap
#[derive(Debug)]
struct PartialMsg {
    inbound: bool,
    created: u64, // first seen
    lasttry: u64, // last retry
    nretries: u32,
    frags: Vec<Vec<u8>>, // missing fragments are zero-sized
}

#[derive(Debug)]
pub(crate) struct FragmentCache {
    partials: BTreeMap<PayloadId, PartialMsg>,
}

impl FragmentCache {
    pub(crate) fn new() -> Self {
        FragmentCache {
            partials: BTreeMap::new(),
        }
    }

    pub(crate) fn add_fragment(&mut self, frag: &LinkFrag, inbound: bool) -> Option<LinkMsg> {
        // Retrieve or create a new PartialMsg
        let partial = self
            .partials
            .entry(PayloadId(frag.payloadid))
            .or_insert_with(|| {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                let frags = vec![Vec::new(); frag.numfrag as usize];
                PartialMsg {
                    inbound,
                    created: timestamp,
                    lasttry: timestamp, // not a retry, but used to schedulefirst retry
                    nretries: 0,
                    frags,
                }
            });

        // Update the fragment
        if frag.fragndx < partial.frags.len() as u32 {
            partial.frags[frag.fragndx as usize] = frag.data.clone();
        } else {
            error!("invalid fragment index: {}", frag.fragndx);
            return None; // Invalid fragment index
        }

        if inbound {
            // Check if all fragments are present
            if partial.frags.iter().all(|f| !f.is_empty()) {
                // Assemble the complete message
                let mut complete_data = Vec::new();
                for fragment in &partial.frags {
                    complete_data.extend_from_slice(fragment);
                }

                // we don't purge the PartialMsg right away so it will be
                // available if other nodes need retries

                // Return the complete LinkMsg
                return Some(LinkMsg {
                    data: complete_data,
                });
            }
        }

        // Return None if the message is not yet complete
        None
    }

    // Return LinkMissing requests for each inbound message that has an
    // overdue fragment and has not been re-requested recently
    pub(crate) fn overdue_missing(&mut self) -> Vec<LinkMissing> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        let mut missing_requests = Vec::new();

        for (&plid, partial) in &mut self.partials {
            if partial.inbound && now >= partial.lasttry + 10 {
                // Collect indices of missing fragments
                let missing_indices: Vec<u32> = partial
                    .frags
                    .iter()
                    .enumerate()
                    .filter(|(_, frag)| frag.is_empty())
                    .map(|(index, _)| index as u32)
                    .collect();

                if !missing_indices.is_empty() {
                    partial.lasttry = now;
                    missing_requests.push(LinkMissing {
                        payloadid: plid.0,
                        fragndx: missing_indices,
                    });
                }
            }
        }

        missing_requests
    }

    // Respond to another node's LinkMissing request by returning
    // any of the needed fragments that we have.
    pub(crate) fn fulfill_missing(&self, missing: LinkMissing) -> Vec<LinkFrag> {
        let mut fragments_to_send = Vec::new();
        if let Some(partial) = self.partials.get(&PayloadId(missing.payloadid)) {
            for &fragndx in &missing.fragndx {
                if let Some(fragment) = partial.frags.get(fragndx as usize) {
                    if !fragment.is_empty() {
                        fragments_to_send.push(LinkFrag {
                            payloadid: missing.payloadid,
                            numfrag: partial.frags.len() as u32,
                            fragndx,
                            data: fragment.clone(),
                        });
                    }
                }
            }
        }
        fragments_to_send
    }
}