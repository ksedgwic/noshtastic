// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use nostr::{self, JsonUtil};
use secp256k1;
use std::convert::TryFrom;
use std::fmt;

use crate::{EncNote, EncString, EncTag, StringType, SyncError};

impl TryFrom<&str> for EncNote {
    type Error = SyncError;

    fn try_from(event_json: &str) -> Result<Self, Self::Error> {
        let evt = nostr::event::Event::from_json(event_json)?;
        EncNote::try_from(evt)
    }
}

impl TryFrom<nostr::event::Event> for EncNote {
    type Error = SyncError;

    fn try_from(evt: nostr::event::Event) -> Result<Self, Self::Error> {
        Ok(EncNote {
            id: evt.id.as_bytes().to_vec(),
            pubkey: evt.pubkey.to_bytes().to_vec(),
            created_at: evt.created_at.as_u64(),
            kind: (evt.kind.as_u16()) as u32,
            tags: evt.tags.into_iter().map(EncTag::from).collect(),
            content: evt.content,
            sig: evt.sig.serialize().to_vec(),
        })
    }
}

impl From<nostr::event::Tag> for EncTag {
    fn from(tag: nostr::event::Tag) -> Self {
        EncTag {
            elements: tag
                .to_vec()
                .iter()
                .map(|s| encode_maybe_hex_string(s))
                .collect(),
        }
    }
}

impl TryFrom<EncNote> for nostr::event::Event {
    type Error = SyncError;

    fn try_from(note: EncNote) -> Result<Self, Self::Error> {
        let id = nostr::event::id::EventId::from_slice(&note.id)?;
        let pubkey = nostr::key::public_key::PublicKey::from_slice(&note.pubkey)?;
        let created_at = note.created_at.into();
        let kind = nostr::event::kind::Kind::from_u16(note.kind as u16);
        let tags = note
            .tags
            .into_iter()
            .map(nostr::event::Tag::try_from)
            .collect::<Result<Vec<nostr::event::Tag>, _>>()?;
        let content = note.content;
        let sig = secp256k1::schnorr::Signature::from_slice(&note.sig)?;
        Ok(nostr::event::Event::new(
            id, pubkey, created_at, kind, tags, content, sig,
        ))
    }
}

impl TryFrom<EncTag> for nostr::event::Tag {
    type Error = SyncError;

    fn try_from(enc_tag: EncTag) -> Result<Self, Self::Error> {
        Ok(nostr::event::Tag::parse(
            enc_tag.elements.iter().map(|e| e.to_string()),
        )?)
    }
}

impl fmt::Display for EncNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let event: nostr::event::Event = self.clone().try_into().map_err(|_| fmt::Error)?;
        write!(f, "{}", event.as_json())
    }
}

fn is_valid_hex_lowercase(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() {
        return None;
    }
    if s.len() % 2 != 0 {
        return None; // Hex strings must have an even number of characters
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return None; // non-lowercase hex chars
    }
    hex::decode(s).ok()
}

fn encode_maybe_hex_string(input: &str) -> EncString {
    if let Some(hex_bytes) = is_valid_hex_lowercase(input) {
        EncString {
            string_type: Some(StringType::HexEncoded(hex_bytes)),
        }
    } else {
        EncString {
            string_type: Some(StringType::Utf8String(input.to_string())),
        }
    }
}

impl fmt::Display for EncString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.string_type {
            Some(StringType::Utf8String(s)) => write!(f, "{}", s),
            Some(StringType::HexEncoded(b)) => write!(f, "{}", hex::encode(b)),
            None => write!(f, ""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::serde_json;
    use prost::Message;
    use std::convert::TryInto;

    #[test]
    fn test_enc_note_conversion() {
        let original_json = r#"{"id":"c190b74042abc315c5880a70f19defc33ec2d8418fed9dde3912a762903319ce","pubkey":"850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc","created_at":1736018180,"kind":1,"tags":[["e","0000c614a9c33ac6e967abba76e04e7948317c131f044ead6d4e03988fa3bbfd","","root"],["e","23cf75046cbf489bddd5920bfb530b30f2ff101eb67b3418711a5ecfd7154c6d"],["e","38c593a0bce380656077f0a5fcbd142cb1d0f82e3d4d42015ce7e6a81e4499a9","","reply"],["p","3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d"],["p","ae1008d23930b776c18092f6eab41e4b09fcf3f03f3641b1b4e6ee3aa166d760"],["p","850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc"]],"content":"Ah, I saw that one ... I'm trying to make one that doesn't rely on mqtt or the internet.  I'm especially concerned that if we have a bridge or gateway to internet sourced notes that we will quickly overwhelm the mesh network.  My hope is that by limiting the \"sync range\" of messages to mesh users in a specific geographic region we can avoid overwhelming the mesh.","sig":"51289738d562bebf32b87cb6bcf5aaa2bb7119a58e19031bda3b1b17bd592891742388aa0a489cd1d60d15498f492cfe54c9bf809cde4fdee9dc52936ab8371d"}"#;

        // {
        //     "id": "c190b74042abc315c5880a70f19defc33ec2d8418fed9dde3912a762903319ce",
        //     "pubkey": "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc",
        //     "created_at": 1736018180,
        //     "kind": 1,
        //     "tags": [
        //         ["e", "0000c614a9c33ac6e967abba76e04e7948317c131f044ead6d4e03988fa3bbfd", "", "root"],
        //         ["e", "23cf75046cbf489bddd5920bfb530b30f2ff101eb67b3418711a5ecfd7154c6d"],
        //         ["e", "38c593a0bce380656077f0a5fcbd142cb1d0f82e3d4d42015ce7e6a81e4499a9", "", "reply"],
        //         ["p", "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d"],
        //         ["p", "ae1008d23930b776c18092f6eab41e4b09fcf3f03f3641b1b4e6ee3aa166d760"],
        //         ["p", "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc"]
        //     ],
        //     "content": "Ah, I saw that one ... I'm trying to make one that doesn't rely on mqtt or the internet.  I'm especially concerned that if we have a bridge or gateway to internet sourced notes that we will quickly overwhelm the mesh network.  My hope is that by limiting the \"sync range\" of messages to mesh users in a specific geographic region we can avoid overwhelming the mesh.",
        //     "sig": "51289738d562bebf32b87cb6bcf5aaa2bb7119a58e19031bda3b1b17bd592891742388aa0a489cd1d60d15498f492cfe54c9bf809cde4fdee9dc52936ab8371d"
        // }

        let note = nostr::event::Event::from_json(original_json).expect("nostr event");

        let enc_note = EncNote::try_from(note).expect("EncNote");

        let mut enc_buffer = Vec::new();
        enc_note.encode(&mut enc_buffer).expect("encoded note");
        println!(
            "original size: {}, encoded size: {}",
            original_json.len(),
            enc_buffer.len(),
        );

        // Convert back from EncNote to JSON
        let event: nostr::event::Event = enc_note
            .try_into()
            .expect("Failed to convert from EncNote to Event");
        let serialized_json = event.as_json();

        // Compare the original and serialized JSON
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(original_json).unwrap(),
            serde_json::from_str::<serde_json::Value>(&serialized_json).unwrap(),
            "The serialized JSON does not match the original JSON"
        );
    }
}
