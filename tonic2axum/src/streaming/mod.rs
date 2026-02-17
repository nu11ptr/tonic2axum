#[cfg(feature = "_client-streaming")]
mod client;

#[cfg(feature = "_server-streaming")]
mod server;

#[cfg(feature = "http-client-streaming")]
pub use client::make_stream_request;

#[cfg(feature = "http-server-streaming")]
pub use server::make_stream_response;

#[cfg(feature = "ws-client-streaming")]
pub use client::{make_ws_stream_request, process_ws_response};
