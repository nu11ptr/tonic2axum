use std::mem;

use axum::Json;
use axum::response::IntoResponse as _;
use serde::Serialize;
use tonic::metadata::MetadataMap;

#[cfg(feature = "_streaming")]
mod streaming;

#[cfg(feature = "http-streaming")]
pub use streaming::{make_stream_request, make_stream_response};

#[cfg(feature = "ws-streaming")]
pub use streaming::{
    close_ws, make_ws_request, make_ws_stream_request, process_ws_response,
    process_ws_stream_response, upgrade_to_ws,
};

/// Converts the parts of an HTTP request into a Tonic request
pub fn make_request<T>(
    headers: http::HeaderMap,
    extensions: http::Extensions,
    message: T,
) -> tonic::Request<T> {
    let metadata = MetadataMap::from_headers(headers);
    tonic::Request::from_parts(metadata, extensions, message)
}

/// Converts a Tonic response into an HTTP response
pub fn make_response<T: Serialize>(
    response: Result<tonic::Response<T>, tonic::Status>,
) -> http::Response<axum::body::Body> {
    match response {
        Ok(response) => make_ok_response(response),
        Err(status) => make_err_response(status),
    }
}

fn make_ok_response<T: Serialize>(
    response: tonic::Response<T>,
) -> http::Response<axum::body::Body> {
    let (meta, message, ext) = response.into_parts();
    let headers = meta.into_headers();

    (http::StatusCode::OK, headers, ext, Json(message)).into_response()
}

fn make_err_response(mut status: tonic::Status) -> http::Response<axum::body::Body> {
    let status_code = match status.code() {
        tonic::Code::Ok => http::StatusCode::OK,
        tonic::Code::Cancelled => http::StatusCode::REQUEST_TIMEOUT,
        tonic::Code::Unknown => http::StatusCode::INTERNAL_SERVER_ERROR,
        tonic::Code::InvalidArgument => http::StatusCode::BAD_REQUEST,
        tonic::Code::DeadlineExceeded => http::StatusCode::GATEWAY_TIMEOUT, // grpc-gateway uses this mapping
        tonic::Code::NotFound => http::StatusCode::NOT_FOUND,
        tonic::Code::AlreadyExists => http::StatusCode::CONFLICT,
        tonic::Code::PermissionDenied => http::StatusCode::FORBIDDEN,
        tonic::Code::ResourceExhausted => http::StatusCode::TOO_MANY_REQUESTS,
        tonic::Code::FailedPrecondition => http::StatusCode::PRECONDITION_FAILED,
        tonic::Code::Aborted => http::StatusCode::CONFLICT,
        tonic::Code::OutOfRange => http::StatusCode::BAD_REQUEST,
        tonic::Code::Unimplemented => http::StatusCode::NOT_IMPLEMENTED,
        tonic::Code::Internal => http::StatusCode::INTERNAL_SERVER_ERROR,
        tonic::Code::Unavailable => http::StatusCode::SERVICE_UNAVAILABLE,
        tonic::Code::DataLoss => http::StatusCode::INTERNAL_SERVER_ERROR,
        tonic::Code::Unauthenticated => http::StatusCode::UNAUTHORIZED,
    };
    let metadata = mem::replace(status.metadata_mut(), MetadataMap::new());
    let headers = metadata.into_headers();

    let mut msg = status.message();
    if msg.is_empty() {
        msg = status.code().description();
    }
    (status_code, headers, msg.to_string()).into_response()
}
