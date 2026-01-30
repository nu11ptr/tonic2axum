use std::{
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use axum_extra::extract::JsonLines;
use futures_core::Stream as _;
use http_body::Frame;
use tonic::{
    codec::{DecodeBuf, Decoder},
    metadata::MetadataMap,
};

// Alternative designs to consider (that are less hacky):
// 1. Use JsonLines<T>, but use something like bitcode in Body to serialize, and then deserialize the T in the Decoder.
// 2. Use a raw axum Body as the request, return the bytes from BodyDataStream in Body trait (using a bytes::Chain to
// prepend a gRPC header), do JSON deserialization in the Decoder.

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

// *** FakeGrpcFrameStreamingHelperInner ***

struct FakeGrpcFrameStreamingHelperInner<T> {
    lines: Pin<Box<JsonLines<T>>>,
    next: Option<Result<T, axum::Error>>,
}

impl<T> FakeGrpcFrameStreamingHelperInner<T> {
    pub fn new(lines: JsonLines<T>) -> Self {
        Self {
            lines: Box::pin(lines),
            next: None,
        }
    }
}

// *** EmptyGrpcFrame ***

#[derive(Clone, Default)]
struct EmptyGrpcFrame([u8; 5]);

impl bytes::Buf for EmptyGrpcFrame {
    fn remaining(&self) -> usize {
        (&self.0[..]).remaining()
    }

    fn chunk(&self) -> &[u8] {
        &self.0[..]
    }

    fn advance(&mut self, cnt: usize) {
        (&self.0[..]).advance(cnt);
    }

    fn copy_to_slice(&mut self, dst: &mut [u8]) {
        (&self.0[..]).copy_to_slice(dst);
    }
}

// *** FakeGrpcFrameStreamingHelper ***

struct FakeGrpcFrameStreamingHelper<T> {
    inner: Arc<Mutex<FakeGrpcFrameStreamingHelperInner<T>>>,
    empty_frame: EmptyGrpcFrame,
}

impl<T> FakeGrpcFrameStreamingHelper<T> {
    pub fn new(lines: JsonLines<T>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeGrpcFrameStreamingHelperInner::new(lines))),
            empty_frame: EmptyGrpcFrame::default(),
        }
    }
}

impl<T> Clone for FakeGrpcFrameStreamingHelper<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            empty_frame: self.empty_frame.clone(),
        }
    }
}

impl<T> tonic::transport::Body for FakeGrpcFrameStreamingHelper<T> {
    type Data = EmptyGrpcFrame;
    type Error = Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut inner = self.inner.lock().expect("poisoned lock");

        // If we already have a buffered item, we don't need to poll again
        if inner.next.is_some() {
            return Poll::Ready(Some(Ok(Frame::data(self.empty_frame.clone()))));
        }

        let lines = inner.lines.as_mut();

        // Poll the stream for the next item
        match lines.poll_next(cx) {
            // Stream has more items
            Poll::Ready(Some(item)) => {
                // Store the item for decode to return
                inner.next = Some(item);
                Poll::Ready(Some(Ok(Frame::data(self.empty_frame.clone()))))
            }
            // Stream ended
            Poll::Ready(None) => Poll::Ready(None),
            // Not ready
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T> Decoder for FakeGrpcFrameStreamingHelper<T> {
    type Item = T;

    type Error = tonic::Status;

    fn decode(&mut self, _src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let mut inner = self.inner.lock().expect("poisoned lock");

        // Return the buffered item if available
        inner
            .next
            .take()
            .transpose()
            .map_err(|e| tonic::Status::internal(e.to_string()))
    }
}
