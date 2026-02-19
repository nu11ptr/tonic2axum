use axum::{
    extract::{
        WebSocketUpgrade,
        ws::{
            CloseFrame, Message, WebSocket,
            close_code::{AGAIN, AWAY, ERROR, INVALID, NORMAL, POLICY, SIZE, UNSUPPORTED},
        },
    },
    response::Response,
};
use bytes::BytesMut;
use futures_core::Stream;
use futures_util::{
    SinkExt as _, StreamExt as _,
    stream::{SplitSink, SplitStream},
};
use serde::{Serialize, de::DeserializeOwned};
use tonic::metadata::MetadataMap;

use crate::streaming::FakeGrpcFrameStreamingHelper;

// *** Upgrade ***

pub async fn upgrade_to_ws<C, Fut>(
    ws_upgrade: WebSocketUpgrade,
    headers: http::HeaderMap,
    extensions: http::Extensions,
    protobuf: bool,
    callback: C,
) -> Response
where
    C: FnOnce(
            http::HeaderMap,
            http::Extensions,
            SplitStream<WebSocket>,
            SplitSink<WebSocket, Message>,
            bool,
        ) -> Fut
        + Send
        + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    ws_upgrade.on_upgrade(move |socket| async move {
        let (sink, stream) = socket.split();
        callback(headers, extensions, stream, sink, protobuf).await;
    })
}

// *** Shared functions ***

async fn send_ws_close_frame(
    ws: &mut SplitSink<WebSocket, Message>,
    status: tonic::Status,
) -> Result<(), axum::Error> {
    let frame = tonic_status_to_ws_close_frame(status);
    ws.send(Message::Close(Some(frame))).await?;
    Ok(())
}

fn tonic_status_to_ws_close_frame(status: tonic::Status) -> CloseFrame {
    let code = match status.code() {
        tonic::Code::Ok => NORMAL,
        tonic::Code::Cancelled => AWAY,
        tonic::Code::InvalidArgument => INVALID,
        tonic::Code::PermissionDenied => POLICY,
        tonic::Code::Unauthenticated => POLICY,
        tonic::Code::ResourceExhausted => SIZE,
        tonic::Code::Unimplemented => UNSUPPORTED,
        tonic::Code::DeadlineExceeded => AGAIN,
        tonic::Code::Unavailable => AGAIN,
        _ => ERROR,
    };
    CloseFrame {
        code,
        reason: status.message().into(),
    }
}

async fn send_ws_msg<T: Send + prost::Message + Serialize + 'static>(
    ws: &mut SplitSink<WebSocket, Message>,
    msg: T,
    protobuf: bool,
) -> Result<(), axum::Error> {
    if protobuf {
        let mut buf = BytesMut::with_capacity(msg.encoded_len());
        msg.encode(&mut buf).map_err(axum::Error::new)?;
        ws.send(Message::Binary(buf.freeze())).await?;
    } else {
        let text = serde_json::to_string(&msg).map_err(axum::Error::new)?;
        ws.send(Message::Text(text.into())).await?;
    }
    Ok(())
}

async fn send_ws_error(ws: &mut SplitSink<WebSocket, Message>, error: axum::Error) {
    tracing::error!("Error processing WS response: {}", error);
    let frame = CloseFrame {
        code: ERROR,
        reason: error.to_string().into(),
    };
    if let Err(err) = ws.send(Message::Close(Some(frame))).await {
        tracing::error!("Error sending close frame error message: {}", err);
    }
}

/// Closes a WebSocket connection with the given tonic status
pub async fn close_ws(mut ws: SplitSink<WebSocket, Message>, status: tonic::Status) {
    if let Err(err) = send_ws_close_frame(&mut ws, status).await {
        tracing::error!("Error sending close frame: {}", err);
    }
}

// *** Client functions ***

/// Converts a web socket request into a Tonic streaming request
pub fn make_ws_stream_request<T: Send + Default + DeserializeOwned + prost::Message + 'static>(
    headers: http::HeaderMap,
    extensions: http::Extensions,
    ws: SplitStream<WebSocket>,
) -> tonic::Request<tonic::Streaming<T>> {
    let metadata = MetadataMap::from_headers(headers);
    // HACK: Unfortunately, Streaming requires a real gRPC frame, so we need to fake it, by returning a fake frame from the
    // Body impl while prepping the real items from the web socket stream to return by Decoder.
    let helper = FakeGrpcFrameStreamingHelper::new(convert_stream(ws));
    // Since we use items polled by Body, but returned by Decoder, we need to clone the helper to use as both parameters
    // to the Streaming constructor.
    let streaming = tonic::Streaming::new_request(helper.clone(), helper, None, None);
    tonic::Request::from_parts(metadata, extensions, streaming)
}

fn convert_stream<T: Send + Default + DeserializeOwned + prost::Message + 'static>(
    ws: SplitStream<WebSocket>,
) -> impl Stream<Item = Result<T, axum::Error>> {
    ws.filter_map(|message| async move {
        match message {
            // Text frame - decode as JSON
            Ok(Message::Text(message)) => {
                Some(serde_json::from_str(&message).map_err(axum::Error::new))
            }
            // Binary frame - decode as protobuf
            Ok(Message::Binary(message)) => Some(T::decode(message).map_err(axum::Error::new)),
            // Close frame with error code
            Ok(Message::Close(Some(close_frame))) => ws_code_to_error(close_frame.code).map(Err),
            // Something else - skip it
            Ok(_) => None,
            // Error - return it
            Err(e) => Some(Err(e)),
        }
    })
}

fn ws_code_to_error(code: u16) -> Option<axum::Error> {
    match code {
        NORMAL => None,
        code => Some(axum::Error::new(format!(
            "WebSocket closed with code: {}",
            code
        ))),
    }
}

/// Processes a Tonic response into a WebSocket response
pub async fn process_ws_response<T: Send + prost::Message + Serialize + 'static>(
    response: Result<tonic::Response<T>, tonic::Status>,
    mut ws: SplitSink<WebSocket, Message>,
    protobuf: bool,
) {
    if let Err(err) = handle_ws_response(response, &mut ws, protobuf).await {
        send_ws_error(&mut ws, err).await;
    }
}

async fn handle_ws_response<T: Send + prost::Message + Serialize + 'static>(
    response: Result<tonic::Response<T>, tonic::Status>,
    ws: &mut SplitSink<WebSocket, Message>,
    protobuf: bool,
) -> Result<(), axum::Error> {
    match response {
        Ok(response) => {
            let msg = response.into_inner();
            send_ws_msg(ws, msg, protobuf).await
        }
        Err(status) => send_ws_close_frame(ws, status).await,
    }
}

// *** Server functions ***

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
