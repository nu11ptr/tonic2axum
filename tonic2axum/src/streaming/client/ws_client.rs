use axum::extract::ws::{
    CloseFrame, Message, WebSocket,
    close_code::{AGAIN, AWAY, ERROR, INVALID, NORMAL, POLICY, SIZE, UNSUPPORTED},
};
use bytes::BytesMut;
use futures_core::Stream;
use futures_util::{
    SinkExt as _, StreamExt as _,
    stream::{SplitSink, SplitStream},
};
use serde::{Serialize, de::DeserializeOwned};
use tonic::metadata::MetadataMap;

use crate::streaming::client::FakeGrpcFrameStreamingHelper;

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
                Some(serde_json::from_str(&message).map_err(|e| axum::Error::new(e)))
            }
            // Binary frame - decode as protobuf
            Ok(Message::Binary(message)) => {
                Some(T::decode(message).map_err(|e| axum::Error::new(e)))
            }
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
    if let Err(e) = handle_ws_response(response, &mut ws, protobuf).await {
        tracing::error!("Error processing WS response: {}", e);
        let frame = CloseFrame {
            code: ERROR,
            reason: e.to_string().into(),
        };
        if let Err(e) = ws.send(Message::Close(Some(frame))).await {
            tracing::error!("Error sending close frame error message: {}", e);
        }
    }
}

async fn handle_ws_response<T: Send + prost::Message + Serialize + 'static>(
    response: Result<tonic::Response<T>, tonic::Status>,
    ws: &mut SplitSink<WebSocket, Message>,
    protobuf: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match response {
        Ok(response) => {
            let msg = response.into_inner();

            if protobuf {
                let mut buf = BytesMut::with_capacity(msg.encoded_len());
                msg.encode(&mut buf)?;
                ws.send(Message::Binary(buf.freeze())).await?;
            } else {
                let text = serde_json::to_string(&msg)?;
                ws.send(Message::Text(text.into())).await?;
            }
        }
        Err(status) => {
            ws.send(Message::Close(Some(tonic_status_to_ws_close_frame(status))))
                .await?;
        }
    }
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
