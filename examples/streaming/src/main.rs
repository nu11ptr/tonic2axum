use std::pin::Pin;

use axum::Router;
use futures_util::Stream;
use futures_util::StreamExt as _;
use tonic::{Request, Response, Status, Streaming, service::Routes};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

use crate::greeter::{
    HelloReply, HelloRequest,
    greeter_axum::make_router,
    greeter_server::{self, GreeterServer},
};

mod greeter {
    tonic::include_proto!("hello.v1");
    tonic::include_proto!("hello.v1.serde");
}

#[derive(Clone)]
struct Greeter;

#[tonic::async_trait]
impl greeter_server::Greeter for Greeter {
    type SayHelloStream = Pin<Box<dyn Stream<Item = Result<HelloReply, Status>> + Send + 'static>>;

    async fn say_hello(
        &self,
        request: Request<Streaming<HelloRequest>>,
    ) -> Result<Response<Self::SayHelloStream>, Status> {
        let stream = request.into_inner();

        // Map the requests into replies lazily to achieve full duplex. For REST, this will likely be
        // used half duplex (depending on the client implementation).
        let stream = stream.map(|request| match request {
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
        });

        Ok(Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() {
    #[derive(OpenApi)]
    #[openapi(tags((name = "Greeter", description = "Greeter API")))]
    struct ApiDoc;

    // Make a router for the generated REST API (using our greeter above)
    let rest_router = make_router(Greeter);
    let (rest_router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest("/v1", rest_router)
        .split_for_parts();

    // Make a router for the gRPC API (using the same greeter, but nested inside a GreeterServer)
    let grpc_router = Routes::new(GreeterServer::new(Greeter))
        .into_axum_router()
        // Don't send stray requests to the gRPC server
        .reset_fallback();

    // Combine the routers into a single router
    let router = Router::new()
        .merge(rest_router)
        .merge(grpc_router)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", api.clone()));

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
