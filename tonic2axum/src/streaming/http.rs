use axum::response::IntoResponse as _;
use axum_extra::extract::JsonLines;
use futures_core::Stream;
use serde::Serialize;
use tonic::metadata::MetadataMap;

use crate::streaming::FakeGrpcFrameStreamingHelper;

// Alternative designs to consider (that are less hacky):
// 1. Use JsonLines<T>, but use something like bitcode in Body to serialize, and then deserialize the T in the Decoder.
//
// 2. Use a raw axum Body as the request, return the bytes from BodyDataStream in Body trait (using a bytes::Chain to
// prepend a gRPC header), do JSON deserialization in the Decoder.
// Update: Actually, #2 is probably not possible without tracking the JSON lines and buffering the input since we would need to
// know how long the input is to prepend the gRPC header.

/// Converts a JSON Lines request into a Tonic streaming request
pub fn make_stream_request<T: Send + 'static>(
    headers: http::HeaderMap,
    extensions: http::Extensions,
    lines: JsonLines<T>,
) -> tonic::Request<tonic::Streaming<T>> {
    let metadata = MetadataMap::from_headers(headers);
    // HACK: Unfortunately, Streaming requires a real gRPC frame, so we need to fake it, by returning a fake frame from the
    // Body impl while prepping the real items from JsonLines to return by Decoder.
    let helper = FakeGrpcFrameStreamingHelper::new(lines);
    // Since we use items polled by Body, but returned by Decoder, we need to clone the helper to use as both parameters
    // to the Streaming constructor.
    let streaming = tonic::Streaming::new_request(helper.clone(), helper, None, None);
    tonic::Request::from_parts(metadata, extensions, streaming)
}

/// Converts a Tonic stream response into a JSON Lines HTTP response
pub fn make_stream_response<S, T>(
    response: Result<tonic::Response<S>, tonic::Status>,
) -> http::Response<axum::body::Body>
where
    S: Stream<Item = Result<T, tonic::Status>> + Send + 'static,
    T: Serialize + Send,
{
    match response {
        Ok(response) => make_ok_stream_response(response),
        Err(status) => crate::make_err_response(status),
    }
}

fn make_ok_stream_response<S, T>(response: tonic::Response<S>) -> http::Response<axum::body::Body>
where
    S: Stream<Item = Result<T, tonic::Status>> + Send + 'static,
    T: Serialize + Send,
{
    let (meta, stream, ext) = response.into_parts();
    let headers = meta.into_headers();
    let lines = JsonLines::new(stream);

    (http::StatusCode::OK, headers, ext, lines).into_response()
}
