use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let tonic_builder = tonic2axum_build::configure_tonic().build_client(false);

    let mut builder = tonic2axum_build::Builder::new()
        .tonic_builder(tonic_builder)
        .custom_state_type("Echo", "crate::Echo")?
        .value_suffix("")
        .type_suffix("")
        .generate_web_sockets(true);

    let (fds, fds_bytes) = builder.compile_protos(&["proto/echo/v1/echo.proto"], &["proto"])?;

    pbjson_build::Builder::new()
        .register_descriptors(&fds_bytes)?
        .build(&[".echo.v1"])?;

    builder.compile_fds(fds, fds_bytes)
}
