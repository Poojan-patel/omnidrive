// omnidrive — entry point.
//
// M2: minimum viable web app. Boot an Axum server on 127.0.0.1:8765 and serve
// a single Askama-rendered page. DB + OAuth arrive in later milestones.

use std::net::SocketAddr;

use anyhow::Result;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod routes;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("omnidrive=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let bind_addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8765".to_string())
        .parse()?;

    let app = routes::router().layer(TraceLayer::new_for_http());

    tracing::info!("omnidrive listening on http://{bind_addr}");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
