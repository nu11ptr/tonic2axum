#[cfg(feature = "http-server-streaming")]
pub mod http_server;

#[cfg(feature = "http-server-streaming")]
pub use http_server::make_stream_response;
