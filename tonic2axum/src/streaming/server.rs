use axum::response::IntoResponse as _;
use axum_extra::extract::JsonLines;
use futures_core::Stream;
use serde::Serialize;

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
