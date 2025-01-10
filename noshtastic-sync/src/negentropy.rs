// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use negentropy::{Bytes, Id, Negentropy, NegentropyStorageVector};
use nostrdb::{Filter, Ndb};

use crate::SyncResult;

#[allow(dead_code)] // FIXME - remove this asap
pub(crate) struct NegentropyState {
    ndb: Ndb,
}

impl NegentropyState {
    pub(crate) fn new(ndb: Ndb) -> Self {
        NegentropyState { ndb }
    }

    fn compose_negentropy(&self) -> SyncResult<Negentropy<NegentropyStorageVector>> {
        let mut storage = NegentropyStorageVector::new();
        let txn = nostrdb::Transaction::new(&self.ndb)?;
        let filters = vec![Filter::new().kinds([1]).build()];
        let notes = self.ndb.query(&txn, &filters, 1024)?;
        debug!("inserting {} notes into negentropy state", notes.len());
        for note in notes {
            match self.ndb.get_note_by_key(&txn, note.note_key) {
                Err(err) => error!("trouble getting note {:?}: {:?}", note.note_key, err),
                Ok(note) => {
                    if let Err(err) = storage.insert(note.created_at(), Id::from_slice(note.id())?)
                    {
                        error!(
                            "trouble inserting note {:064} into negentropy storage: {:?}",
                            hex::encode(note.id()),
                            err
                        );
                    }
                }
            }
        }
        storage.seal()?;
        Ok(Negentropy::new(storage, 200)?)
    }

    pub(crate) fn initiate(&mut self) -> SyncResult<Vec<u8>> {
        debug!("initiate starting");
        let mut negentropy = self.compose_negentropy()?;
        let negmsg = negentropy.initiate()?;
        negentropy.dump_query(&negmsg, std::io::stdout())?;
        Ok(negmsg.to_bytes())
    }

    pub(crate) fn reconcile(
        &self,
        inmsg: &[u8],
        have_ids: &mut Vec<Vec<u8>>,
        need_ids: &mut Vec<Vec<u8>>,
    ) -> SyncResult<Option<Vec<u8>>> {
        debug!("reconcile starting");
        let mut negentropy = self.compose_negentropy()?;
        negentropy.dump_query(&Bytes::from_slice(inmsg), std::io::stdout())?;
        negentropy.set_initiator();
        let mut have_ids_tmp: Vec<negentropy::Id> = Vec::new();
        let mut need_ids_tmp: Vec<negentropy::Id> = Vec::new();
        let maybe_negmsg = negentropy.reconcile_with_ids(
            &Bytes::from_slice(inmsg),
            &mut have_ids_tmp,
            &mut need_ids_tmp,
        )?;
        *have_ids = have_ids_tmp
            .into_iter()
            .map(|id| id.to_bytes().to_vec())
            .collect();
        *need_ids = need_ids_tmp
            .into_iter()
            .map(|id| id.to_bytes().to_vec())
            .collect();
        if let Some(negmsg) = &maybe_negmsg {
            negentropy.dump_query(negmsg, std::io::stdout())?;
        }
        Ok(maybe_negmsg.map(|bytes| bytes.to_vec()))
    }
}
