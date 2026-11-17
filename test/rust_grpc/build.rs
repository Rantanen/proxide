fn main() -> Result<(), Box<dyn std::error::Error>>
{
    // The descriptor set is served by the gRPC reflection service.
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("rust_grpc_descriptor.bin"))
        // reflection_test.proto is only served through reflection, there's no service for it.
        .compile(
            &["proto/rust_grpc.proto", "proto/reflection_test.proto"],
            &["proto"],
        )?;
    Ok(())
}
