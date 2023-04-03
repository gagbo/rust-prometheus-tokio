use axum::{routing::get, Router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Add the Tokio collector to the default registry
    // You can also use any registry you setup inside a lazy_static or a once_cell.
    prometheus::default_registry()
        .register(Box::new(prometheus_tokio::TokioCollector::for_self()))
        .unwrap();

    let app = Router::new()
        .route("/", get(root))
        .route("/metrics", get(metrics));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

// Encode the metrics with the same process as when using Prometheus
async fn metrics() -> Result<String, String> {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&prometheus::default_registry().gather(), &mut buffer) {
        return Err(format!("could not encode custom metrics: {e}"));
    };
    String::from_utf8(buffer.clone())
        .map_err(|e| format!("custom metrics could not be from_utf8'd: {e}"))
}
