// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{proto::LinkMissing, LinkFrag, LinkMsg, MsgId};

#[derive(Debug, Clone)]
struct CachedFrag {
    rotoff: u8,    // octet rotation offset for next send (see noshtastic:#15)
    data: Vec<u8>, // data w/o any rotation
}

impl CachedFrag {
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn rotate_octets(&mut self) -> (u8, Vec<u8>) {
        let rotoff = self.rotoff;
        self.rotoff = self.rotoff.wrapping_add(1);
        (
            rotoff,
            self.data
                .iter()
                .map(|&byte| byte.wrapping_add(rotoff))
                .collect(),
        )
    }

    fn unrotate_octets(data: Vec<u8>, rotoff: u8) -> Vec<u8> {
        data.into_iter()
            .map(|byte| byte.wrapping_sub(rotoff))
            .collect()
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct PartialMsg {
    inbound: bool,
    created: u64, // first seen
    lasttry: u64, // last retry
    nretries: u32,
    completed: bool, // don't need to upcall to client more than once
    frags: Vec<CachedFrag>,
}

#[derive(Debug)]
pub(crate) struct FragmentCache {
    partials: BTreeMap<MsgId, PartialMsg>,
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
            .entry(MsgId::new(frag.msgid, None))
            .or_insert_with(|| {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                let frags = vec![
                    CachedFrag {
                        rotoff: 0,
                        data: Vec::new(),
                    };
                    frag.numfrag as usize
                ];
                PartialMsg {
                    inbound,
                    created: timestamp,
                    lasttry: timestamp, // not a retry, but used to schedulefirst retry
                    nretries: 0,
                    frags,
                    completed: false,
                }
            });

        // Update the fragment
        if frag.fragndx < partial.frags.len() as u32 {
            // Unrotate the data before storing it in the fragment
            let unrotated_data = CachedFrag::unrotate_octets(frag.data.clone(), frag.rotoff as u8);
            partial.frags[frag.fragndx as usize].data = unrotated_data;
        } else {
            error!("invalid fragment index: {}", frag.fragndx);
            return None; // Invalid fragment index
        }

        if !partial.completed && inbound {
            // Check if all fragments are present
            if partial.frags.iter().all(|f| !f.is_empty()) {
                // Assemble the complete message
                let mut complete_data = Vec::new();
                for fragment in &partial.frags {
                    complete_data.extend_from_slice(&fragment.data);
                }

                // we don't purge the PartialMsg right away so it will be
                // available if other nodes need retries

                partial.completed = true;
                return Some(LinkMsg {
                    msgid: frag.msgid,
                    data: complete_data,
                });
            }
        }

        // Return None if the message is not yet complete or has
        // already been returned previously as completed
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
        let overdue_seconds = 60;

        for (&msgid, partial) in &mut self.partials {
            if partial.inbound && now >= partial.lasttry + overdue_seconds {
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
                        msgid: msgid.base,
                        fragndx: missing_indices,
                    });
                }
            }
        }

        missing_requests
    }

    // Respond to another node's LinkMissing request by returning
    // any of the needed fragments that we have.
    pub(crate) fn fulfill_missing(&mut self, missing: LinkMissing) -> Vec<LinkFrag> {
        let mut fragments_to_send = Vec::new();

        // Retrieve the PartialMsg if it exists
        if let Some(partial) = self.partials.get_mut(&MsgId::new(missing.msgid, None)) {
            for &fragndx in &missing.fragndx {
                // Safely access the fragment by index
                if let Some(fragment) = partial.frags.get_mut(fragndx as usize) {
                    // Skip empty fragments
                    if fragment.is_empty() {
                        continue;
                    }

                    // Rotate the octets and prepare the fragment for sending
                    let (rotoff, data) = fragment.rotate_octets();
                    fragments_to_send.push(LinkFrag {
                        msgid: missing.msgid,
                        numfrag: partial.frags.len() as u32,
                        fragndx,
                        rotoff: rotoff as u32,
                        data,
                    });
                }
            }
        }

        fragments_to_send
    }
}
