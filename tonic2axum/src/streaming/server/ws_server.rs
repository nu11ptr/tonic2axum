use axum::extract::ws::{Message, WebSocket, close_code::NORMAL};
use futures_core::Stream;
use futures_util::{
    StreamExt as _,
    stream::{SplitSink, SplitStream},
};
use serde::{Serialize, de::DeserializeOwned};
use tonic::metadata::MetadataMap;

use crate::streaming::ws::{send_ws_close_frame, send_ws_error, send_ws_msg};

/// Processes a Tonic stream response into a WebSocket response
pub async fn process_ws_stream_response<S, T>(
    response: Result<tonic::Response<S>, tonic::Status>,
    mut ws: SplitSink<WebSocket, Message>,
    protobuf: bool,
) where
    S: Stream<Item = Result<T, tonic::Status>> + Send + Unpin + 'static,
    T: prost::Message + Serialize + Send + 'static,
{
    if let Err(err) = handle_ws_stream_response(response, &mut ws, protobuf).await {
        send_ws_error(&mut ws, err).await;
    }
}

async fn handle_ws_stream_response<S, T>(
    response: Result<tonic::Response<S>, tonic::Status>,
    ws: &mut SplitSink<WebSocket, Message>,
    protobuf: bool,
) -> Result<(), axum::Error>
where
    S: Stream<Item = Result<T, tonic::Status>> + Send + Unpin + 'static,
    T: prost::Message + Serialize + Send + 'static,
{
    match response {
        Ok(response) => {
            let mut stream = response.into_inner();
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(msg) => send_ws_msg(ws, msg, protobuf).await?,
                    Err(status) => send_ws_close_frame(ws, status).await?,
                }
            }
        }
        Err(status) => send_ws_close_frame(ws, status).await?,
    }
    Ok(())
}

/// Converts a WebSocket message into a Tonic request
pub async fn make_ws_request<T: Send + Default + prost::Message + DeserializeOwned + 'static>(
    headers: http::HeaderMap,
    extensions: http::Extensions,
    mut ws: SplitStream<WebSocket>,
) -> Option<tonic::Request<T>> {
    let metadata = MetadataMap::from_headers(headers);

    while let Some(message) = ws.next().await {
        match convert_ws_to_item(message) {
            // Item received - return it
            Ok((Some(item), _)) => {
                return Some(tonic::Request::from_parts(metadata, extensions, item));
            }
            // Normal end of stream - all done
            Ok((None, true)) => return None,
            // No-op - continue
            Ok((None, false)) => continue,
            // Error - all done
            Err(e) => {
                tracing::error!("Error converting WS message to item: {}", e);
                return None;
            }
        }
    }

    None
}

fn convert_ws_to_item<T: Send + Default + prost::Message + DeserializeOwned + 'static>(
    result: Result<Message, axum::Error>,
) -> Result<(Option<T>, bool), axum::Error> {
    match result {
        // Text frame - decode as JSON
        Ok(Message::Text(message)) => serde_json::from_str(&message)
            .map(|msg| (Some(msg), false))
            .map_err(axum::Error::new),
        // Binary frame - decode as protobuf
        Ok(Message::Binary(message)) => T::decode(message)
            .map(|msg| (Some(msg), false))
            .map_err(axum::Error::new),
        // Normal end of stream
        Ok(Message::Close(Some(close_frame))) if close_frame.code == NORMAL => Ok((None, true)),
        // Normal end of stream
        Ok(Message::Close(None)) => Ok((None, true)),
        // Close frame with error code
        Ok(Message::Close(Some(close_frame))) => Err(axum::Error::new(format!(
            "WebSocket closed with code: {}",
            close_frame.code
        ))),
        // Something else - skip it
        Ok(_) => Ok((None, false)),
        // Error - return it
        Err(e) => Err(e),
    }
}
