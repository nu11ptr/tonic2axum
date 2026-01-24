use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let fds_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("fds.bin");

    // 1. Prost_build: Configure to save file descriptor set to a file, compile the proto files and read the file descriptor set from the file.
    let mut config = prost_build::Config::new();
    config.file_descriptor_set_path(&fds_path);
    let fds = config.load_fds(&["proto/hello/v1/hello.proto"], &["proto"])?;
    let bytes = std::fs::read(&fds_path)?;

    // 2. Tonic_prost_build: Configure the Builder
    let builder = tonic_prost_build::configure().build_client(false);

    // 3. Tonic2axum_build: We wrap the tonic_prost_build service generator in our builder, and use our service generator with prost_build.
    let rest_generator = tonic2axum_build::Generator::new(builder.service_generator(), &bytes)?;
    config.service_generator(Box::new(rest_generator));

    // 4. Prost_build: Generate the Rust files
    config.compile_fds(fds)?;

    Ok(())
}
