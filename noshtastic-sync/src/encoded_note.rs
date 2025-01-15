// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use brotli::{CompressorWriter, DecompressorWriter};
use nostr::{self, JsonUtil};
use secp256k1;
use std::convert::TryFrom;
use std::fmt;
use std::io::Write;

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
            content: Some(encode_maybe_hex_string(&evt.content)),
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
        let content = note
            .content
            .map(|enc_string| EncString::to_string(&enc_string))
            .unwrap_or("".to_string());
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

fn compress_brotli(input: &str) -> Vec<u8> {
    let mut compressed = Vec::new();
    {
        let mut compressor = CompressorWriter::new(&mut compressed, 4096, 11, 22);
        compressor.write_all(input.as_bytes()).unwrap();
    }
    compressed
}

fn decompress_brotli(input: &[u8]) -> String {
    let mut decompressed = Vec::new();
    {
        let mut decompressor = DecompressorWriter::new(&mut decompressed, 4096);
        decompressor.write_all(input).unwrap();
    }
    String::from_utf8(decompressed).unwrap()
}

fn encode_maybe_hex_string(input: &str) -> EncString {
    if let Some(hex_bytes) = is_valid_hex_lowercase(input) {
        EncString {
            string_type: Some(StringType::HexEncoded(hex_bytes)),
        }
    } else if input.len() > 50 {
        // Compress if input is longer than 50 bytes
        let compressed = compress_brotli(input);
        EncString {
            string_type: Some(StringType::Compressed(compressed)),
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
            Some(StringType::Compressed(b)) => {
                let decompressed = decompress_brotli(b);
                write!(f, "{}", decompressed)
            }
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

    // the test notes are pretty printed, but use compact json for size metrics
    fn compact_json(pretty_json: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(pretty_json).expect("Invalid JSON");
        serde_json::to_string(&value).expect("Failed to serialize JSON")
    }

    #[test]
    fn test_enc_notes_conversion() {
        let test_notes = [
            // Longer content string:
            r#"{
              "id": "c190b74042abc315c5880a70f19defc33ec2d8418fed9dde3912a762903319ce",
              "pubkey": "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc",
              "created_at": 1736018180,
              "kind": 1,
              "tags": [
                [
                  "e",
                  "0000c614a9c33ac6e967abba76e04e7948317c131f044ead6d4e03988fa3bbfd",
                  "",
                  "root"
                ],
                [
                  "e",
                  "23cf75046cbf489bddd5920bfb530b30f2ff101eb67b3418711a5ecfd7154c6d"
                ],
                [
                  "e",
                  "38c593a0bce380656077f0a5fcbd142cb1d0f82e3d4d42015ce7e6a81e4499a9",
                  "",
                  "reply"
                ],
                [
                  "p",
                  "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d"
                ],
                [
                  "p",
                  "ae1008d23930b776c18092f6eab41e4b09fcf3f03f3641b1b4e6ee3aa166d760"
                ],
                [
                  "p",
                  "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc"
                ]
              ],
              "content": "Ah, I saw that one ... I'm trying to make one that doesn't rely on mqtt or the internet.  I'm especially concerned that if we have a bridge or gateway to internet sourced notes that we will quickly overwhelm the mesh network.  My hope is that by limiting the \"sync range\" of messages to mesh users in a specific geographic region we can avoid overwhelming the mesh.",
              "sig": "51289738d562bebf32b87cb6bcf5aaa2bb7119a58e19031bda3b1b17bd592891742388aa0a489cd1d60d15498f492cfe54c9bf809cde4fdee9dc52936ab8371d"
            }"#,
            //
            // Empty content:
            r#"{
              "id": "51f56cbaa70efb98995616fe73fcbf5169de8a7d9b50140bbace6f0f30f93c6e",
              "pubkey": "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc",
              "created_at": 1729536636,
              "kind": 1,
              "tags": [
                [
                  "e",
                  "278c491bc92001ce9223110bb76dc05c8b7aa30cd6bef1fa106c919249f72827",
                  "",
                  "root"
                ],
                [
                  "p",
                  "f147e87ccf16ed5bb3a91de5538af015410db86c41373e0b3933ae4c905cd7ff"
                ]
              ],
              "content": "",
              "sig": "b818e303b124eedcb3fc5b8aa1b4d2b49b19fa0a9be87e6761d905783c52d397adaad666d7e57be9426026bba408d28aa2e0a9272ba6b82ef22805aff63a04b1"
            }"#,
            //
            // Too bad this doesn't fit in 200 ..
            r#"{
              "id": "3127fa5371d17771514829bd4067003dadac0f847d3055c5e4a6fd560c3467f9",
              "pubkey": "850605096dbfb50b929e38a6c26c3d56c425325c85e05de29b759bc0e5d6cebc",
              "created_at": 1728398992,
              "kind": 1,
              "tags": [],
              "content": "When’s Trump gonna step in and fix this hurricane with a Sharpie again? 🤔",
              "sig": "87b1a2a49fe83afa7436917c3a38a105fbaf281f857be64e68bcdb62ad9d47790211a02ee3ac1f8ca33b05f567e744593af565202e0a17e74be2e5826a206857"
            }"#,
            //
            // Very short:
            r#"{
              "content": "Testing 4, 5, 6",
              "created_at": 1732223844,
              "id": "ead34ac6ce79ce4d9d512508cc00d9434a592c7ef4b767f89b6512b0640ed913",
              "kind": 1,
              "pubkey": "dcdc0e77fe223f3f62a476578350133ca97767927df676ca7ca7b92a413a7703",
              "sig": "208b6f78348d922ab7cf7baf527800fb79d979864e523e99e9cbc45c48611e51df472cfad50376e744d7e575be47a87394069820284a30a2af02081a345d3bf3",
              "tags": []
            }"#,
        ];

        for (index, original_json) in test_notes.iter().enumerate() {
            let compacted_json = compact_json(original_json);
            let note = nostr::event::Event::from_json(original_json).expect("nostr event");
            let enc_note = EncNote::try_from(note).expect("EncNote");

            let mut enc_buffer = Vec::new();
            enc_note.encode(&mut enc_buffer).expect("encoded note");
            println!(
                "Test case {}: original size: {}, encoded size: {}",
                index + 1,
                compacted_json.len(),
                enc_buffer.len()
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
                "The serialized JSON does not match the original JSON for test case {}",
                index + 1
            );
        }
    }
}
