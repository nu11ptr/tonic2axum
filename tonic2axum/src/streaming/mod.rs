#[cfg(feature = "client-streaming")]
mod client;

#[cfg(feature = "server-streaming")]
mod server;

#[cfg(feature = "client-streaming")]
pub use client::make_stream_request;

#[cfg(feature = "server-streaming")]
pub use server::make_stream_response;
