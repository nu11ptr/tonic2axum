use std::{
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use futures_core::Stream;
use http_body::Frame;
use tonic::codec::{DecodeBuf, Decoder};

#[cfg(feature = "http-streaming")]
mod http;

#[cfg(feature = "ws-streaming")]
mod ws;

#[cfg(feature = "http-streaming")]
pub use http::{make_stream_request, make_stream_response};

#[cfg(feature = "ws-streaming")]
pub use ws::{
    close_ws, make_ws_request, make_ws_stream_request, process_ws_response,
    process_ws_stream_response, upgrade_to_ws,
};

// *** FakeGrpcFrameStreamingHelperInner ***

struct FakeGrpcFrameStreamingHelperInner<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
    lines: Pin<Box<S>>,
    next: Option<Result<T, axum::Error>>,
}

impl<S, T> FakeGrpcFrameStreamingHelperInner<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
    pub fn new(stream: S) -> Self {
        Self {
            lines: Box::pin(stream),
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

struct FakeGrpcFrameStreamingHelper<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
    inner: Arc<Mutex<FakeGrpcFrameStreamingHelperInner<S, T>>>,
    empty_frame: EmptyGrpcFrame,
}

impl<S, T> FakeGrpcFrameStreamingHelper<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
    pub fn new(lines: S) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeGrpcFrameStreamingHelperInner::new(lines))),
            empty_frame: EmptyGrpcFrame::default(),
        }
    }
}

impl<S, T> Clone for FakeGrpcFrameStreamingHelper<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            empty_frame: self.empty_frame.clone(),
        }
    }
}

impl<S, T> tonic::transport::Body for FakeGrpcFrameStreamingHelper<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
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

impl<S, T> Decoder for FakeGrpcFrameStreamingHelper<S, T>
where
    S: Stream<Item = Result<T, axum::Error>> + Send + 'static,
    T: Send + 'static,
{
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

#[cfg(test)]
mod tests {
    use axum::extract::FromRequest as _;
    use axum_extra::extract::JsonLines;
    use bytes::Bytes;
    use futures_util::StreamExt as _;
    use serde::{Deserialize, Serialize};

    use crate::make_stream_request;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestRequest {
        id: u32,
    }

    impl From<TestRequest> for Bytes {
        fn from(item: TestRequest) -> Self {
            let mut line = serde_json::to_string(&item).unwrap();
            line.push('\n');
            line.into()
        }
    }

    #[tokio::test]
    async fn test_make_stream_request_roundtrip() {
        let requests = vec![
            TestRequest { id: 1 },
            TestRequest { id: 2 },
            TestRequest { id: 3 },
        ];
        let cloned_requests = requests.clone();
        let stream = async_stream::stream! {
            for req in cloned_requests {
                yield Ok::<_, tonic::Status>(req);
            }
        };

        // We have to build JsonLines this way because it has to be built from a request to get the correct type parameters.
        let body = axum::body::Body::from_stream(stream);
        let request = http::Request::new(body);
        let json_lines: JsonLines<TestRequest> =
            JsonLines::from_request(request, &()).await.unwrap();
        let headers = http::HeaderMap::new();
        let extensions = http::Extensions::new();

        let request = make_stream_request(headers, extensions, json_lines);

        let mut streaming = request.into_inner();
        let mut received_items = Vec::with_capacity(requests.len());

        while let Some(result) = streaming.next().await {
            match result {
                Ok(item) => received_items.push(item),
                Err(status) => panic!("Unexpected error: {}", status),
            }
        }

        assert_eq!(received_items, requests);
    }
}
