fn main() {
    tonic_build::configure()
        .build_server(false) // Don't build gRPC server if not needed
        .out_dir("protos") // Specify the directory for generated files
        .compile(&["protos/link.proto"], &["protos"])
        .unwrap();
}
