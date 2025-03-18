// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use log::*;
use meshtastic::{
    api::{ConnectedStreamApi, StreamApi},
    packet::PacketReceiver,
    utils,
};

use crate::{LinkError, LinkResult};

use meshtastic::utils::stream::BleId;

pub(crate) async fn create_ble_stream(
    maybe_hint: &Option<String>,
) -> LinkResult<(
    PacketReceiver,
    ConnectedStreamApi<meshtastic::api::state::Connected>,
)> {
    debug!("build_ble_stream starting, hint {:?}", maybe_hint);
    if maybe_hint.is_none() {
        return Err(LinkError::internal_error(
            "scan for devices not yet implemented; use mac addr hint".to_string(),
        ));
    }
    let ble_stream =
        utils::stream::build_ble_stream(&BleId::from_mac_address(maybe_hint.as_ref().unwrap())?)
            .await?;
    debug!("build_ble_stream finished");

    let stream_api = StreamApi::new();
    let (mesh_in_rx, stream_api) = stream_api.connect(ble_stream).await;
    debug!("ble_stream connected");

    Ok((mesh_in_rx, stream_api))
}
