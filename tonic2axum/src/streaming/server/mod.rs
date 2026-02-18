#[cfg(feature = "http-server-streaming")]
mod http_server;

#[cfg(feature = "ws-server-streaming")]
mod ws_server;

#[cfg(feature = "http-server-streaming")]
pub use http_server::make_stream_response;

#[cfg(feature = "ws-server-streaming")]
pub use ws_server::{make_ws_request, process_ws_stream_response};
