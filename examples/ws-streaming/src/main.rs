use std::pin::Pin;
use std::time::Duration;

use axum::Router;
use futures_util::{Stream, StreamExt as _};
use rand::Rng as _;
use tokio::time::sleep;
use tonic::{Request, Response, Status, Streaming, service::Routes};

use crate::echo::{
    EchoReply, EchoRequest,
    echo_axum::make_router,
    echo_server::{self, EchoServer},
};

mod echo {
    tonic::include_proto!("echo.v1");
    tonic::include_proto!("echo.v1.serde");
}

#[derive(Clone)]
struct Echo;

#[tonic::async_trait]
impl echo_server::Echo for Echo {
    type BidiEchoStream = Pin<Box<dyn Stream<Item = Result<EchoReply, Status>> + Send + 'static>>;

    async fn bidi_echo(
        &self,
        request: Request<Streaming<EchoRequest>>,
    ) -> Result<Response<Self::BidiEchoStream>, Status> {
        let mut client_stream = request.into_inner();

        // Echo back each client message, interleaved with periodic server greetings.
        // Stops when the client disconnects.
        let output = async_stream::stream! {
            loop {
                let delay = rand::rng().random_range(5..=10);
                tokio::select! {
                    msg = client_stream.next() => match msg {
                        Some(Ok(req)) => yield Ok(EchoReply {
                            message: format!("echo: {}", req.message),
                        }),
                        Some(Err(status)) => {
                            yield Err(status);
                            break;
                        }
                        None => break,
                    },
                    _ = sleep(Duration::from_secs(delay)) => {
                        yield Ok(EchoReply {
                            message: "hello, this is the server!".to_string(),
                        });
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(output)))
    }
}

#[tokio::main]
async fn main() {
    // REST + WebSocket router
    let rest_router = make_router(Echo);

    // gRPC router
    let grpc_router = Routes::new(EchoServer::new(Echo))
        .into_axum_router()
        .reset_fallback();

    let router = Router::new().merge(rest_router).merge(grpc_router);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000")
        .await
        .unwrap();
    println!(
        "WS streaming gRPC/REST server listening on {}",
        listener.local_addr().unwrap()
    );
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("error: {}", e);
    }
}
