#[cfg(feature = "_client-streaming")]
mod client;

#[cfg(feature = "_server-streaming")]
mod server;

#[cfg(any(feature = "ws-client-streaming", feature = "ws-server-streaming"))]
mod ws;

#[cfg(feature = "http-client-streaming")]
pub use client::make_stream_request;

#[cfg(feature = "http-server-streaming")]
pub use server::make_stream_response;

#[cfg(feature = "ws-client-streaming")]
pub use client::{make_ws_stream_request, process_ws_response};

#[cfg(feature = "ws-server-streaming")]
pub use server::{make_ws_request, process_ws_stream_response};
