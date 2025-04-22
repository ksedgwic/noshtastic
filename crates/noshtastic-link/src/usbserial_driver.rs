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
use std::process;

use crate::{LinkError, LinkResult};

pub(crate) async fn create_usbserial_stream(
    maybe_serial: &Option<String>,
) -> LinkResult<(
    PacketReceiver,
    ConnectedStreamApi<meshtastic::api::state::Connected>,
)> {
    let serial = match maybe_serial.clone() {
        Some(serial) => serial, // specified, just use
        None => {
            debug!("querying available serial ports ...");
            let available_ports = utils::stream::available_serial_ports()?;

            match available_ports.as_slice() {
                [port] => port.clone(), // exactly one port available
                [] => {
                    return Err(LinkError::missing_parameter(
                        "No available serial ports found. Use --serial to specify.".to_string(),
                    ));
                }
                _ => {
                    return Err(LinkError::missing_parameter(format!(
                        "Multiple available serial ports found: {:?}. Use --serial to specify.",
                        available_ports
                    )));
                }
            }
        }
    };

    info!("opening serial link on {}", serial);

    let serial_stream = utils::stream::build_serial_stream(serial.clone(), None, None, None)
        .map_err(|err| {
            error!("failed to open {}: {:?}", serial, err);
            info!(
                "available ports: {:?}. Use --serial to specify.",
                utils::stream::available_serial_ports().expect("available_serial_ports")
            );
            process::exit(1);
        })
        .unwrap();

    let stream_api = StreamApi::new();
    let (mesh_in_rx, stream_api) = stream_api.connect(serial_stream).await;
    debug!("usbserial_stream connected");

    Ok((mesh_in_rx, stream_api))
}
