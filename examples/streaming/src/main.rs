use axum::Router;
use futures_util::stream::iter;
use futures_util::{StreamExt as _, stream::Iter};
use tonic::{Request, Response, Status, Streaming, service::Routes};

use crate::greeter::{
    HelloReply, HelloRequest,
    greeter_server::{self, GreeterServer},
    make_greeter_router,
};

mod greeter {
    tonic::include_proto!("hello.v1");
    tonic::include_proto!("hello.v1.serde");
}

#[derive(Clone)]
struct Greeter;

#[tonic::async_trait]
impl greeter_server::Greeter for Greeter {
    // WARNING: Notice we read all requests BEFORE potentially sending replies back. This is intentional. A full duplex
    // method will work for gRPC, but might not work for REST. Half duplex is okay for both. It is best to avoid bidirectional
    // streaming when possible for this reason. Using client *OR* server streaming should be fine.

    type SayHelloStream = Iter<std::vec::IntoIter<Result<HelloReply, Status>>>;

    async fn say_hello(
        &self,
        request: Request<Streaming<HelloRequest>>,
    ) -> Result<Response<Self::SayHelloStream>, Status> {
        let stream = request.into_inner();

        // Collect and transform all the requests into replies first
        let stream = stream
            .map(|request| match request {
                Ok(request) => {
                    let HelloRequest {
                        first_name,
                        last_name,
                    } = request;

                    Ok(HelloReply {
                        message: format!("Hello, {} {}!", first_name, last_name),
                    })
                }
                Err(status) => Err(status),
            })
            .collect::<Vec<_>>()
            .await;

        // After all the requests are collected and transformed into replies, send the replies back as a new stream
        Ok(Response::new(iter(stream)))
    }
}

#[tokio::main]
async fn main() {
    // Make a router for the generated REST API (using our greeter above)
    let rest_router = make_greeter_router(Greeter);

    // Make a router for the gRPC API (using the same greeter, but nested inside a GreeterServer)
    let grpc_router = Routes::new(GreeterServer::new(Greeter))
        .into_axum_router()
        // Don't send stray requests to the gRPC server
        .reset_fallback();

    // Combine the routers into a single router
    let router = Router::new().nest("/v1", rest_router).merge(grpc_router);

    // Bind to a port and start the server using our combined router
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000")
        .await
        .unwrap();
    println!(
        "Streaming gRPC/REST server listening on {}",
        listener.local_addr().unwrap()
    );
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("error: {}", e);
    }
}
