use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // 1. tonic_prost_build: Configure the Builder
    let tonic_builder = tonic2axum_build::configure_tonic().build_client(false);

    // 2. tonic2axum_build: Configure the Builder
    let mut builder = tonic2axum_build::Builder::new()
        .tonic_builder(tonic_builder)
        .custom_state_type("Greeter", "crate::Greeter")?
        .generate_openapi(true);

    // 3. tonic2axum_build: Compile the proto files and return the file descriptor set and its raw bytes.
    let (fds, fds_bytes) = builder.compile_protos(&["proto/hello/v1/hello.proto"], &["proto"])?;

    // 4. pbjson_build: Implement Serialization and Deserialization for the messages
    pbjson_build::Builder::new()
        .register_descriptors(&fds_bytes)?
        .build(&[".hello.v1"])?;

    // 5. tonic2axum_build: Generate the code (Prost, Tonic, and Axum)
    builder.compile_fds(fds, fds_bytes)
}
