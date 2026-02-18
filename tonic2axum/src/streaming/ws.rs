use axum::extract::ws::{
    CloseFrame, Message, WebSocket,
    close_code::{AGAIN, AWAY, ERROR, INVALID, NORMAL, POLICY, SIZE, UNSUPPORTED},
};
use bytes::BytesMut;
use futures_util::{SinkExt as _, stream::SplitSink};
use serde::Serialize;

pub(crate) async fn send_ws_close_frame(
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

pub(crate) async fn send_ws_msg<T: Send + prost::Message + Serialize + 'static>(
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

pub(crate) async fn send_ws_error(ws: &mut SplitSink<WebSocket, Message>, error: axum::Error) {
    tracing::error!("Error processing WS response: {}", error);
    let frame = CloseFrame {
        code: ERROR,
        reason: error.to_string().into(),
    };
    if let Err(err) = ws.send(Message::Close(Some(frame))).await {
        tracing::error!("Error sending close frame error message: {}", err);
    }
}
