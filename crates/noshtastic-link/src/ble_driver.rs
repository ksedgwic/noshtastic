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

pub fn parse_ble_id(s: &str) -> LinkResult<BleId> {
    if let Some(name_part) = s.strip_prefix("name=") {
        // e.g. "name=KitchenSensor" -> "KitchenSensor"
        Ok(BleId::from_name(name_part))
    } else if let Some(mac_part) = s.strip_prefix("MAC=") {
        // e.g. "MAC=64:E8:33:47:07:C1" -> "64:E8:33:47:07:C1"
        Ok(BleId::from_mac_address(mac_part)?)
    } else {
        Err(LinkError::invalid_argument(
            "Missing 'name=' or 'MAC=' prefix".to_string(),
        ))
    }
}

pub(crate) async fn create_ble_stream(
    maybe_hint: &Option<String>,
) -> LinkResult<(
    PacketReceiver,
    ConnectedStreamApi<meshtastic::api::state::Connected>,
)> {
    info!("build_ble_stream starting, hint {:?}", maybe_hint);
    if maybe_hint.is_none() {
        return Err(LinkError::internal_error(
            "need a BLE device hint".to_string(),
        ));
    }
    let ble_stream =
        utils::stream::build_ble_stream(&parse_ble_id(maybe_hint.as_ref().unwrap())?).await?;
    debug!("build_ble_stream finished");

    let stream_api = StreamApi::new();
    let (mesh_in_rx, stream_api) = stream_api.connect(ble_stream).await;
    info!("ble_stream connected");

    Ok((mesh_in_rx, stream_api))
}

pub async fn scan_for_ble_radios() -> LinkResult<Vec<String>> {
    Ok(utils::stream::available_ble_radios()
        .await?
        .iter()
        .map(|bleid| bleid.to_string())
        .collect())
}
