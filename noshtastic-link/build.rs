// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

fn main() {
    tonic_build::configure()
        .build_server(false) // Don't build gRPC server if not needed
        .out_dir("protos") // Specify the directory for generated files
        .compile(&["protos/link.proto"], &["protos"])
        .unwrap();
}
