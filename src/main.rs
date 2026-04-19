// omnidrive — entry point.
//
// M3: wire up SQLite. Open the database at startup, run migrations, stash the
// connection in AppState so handlers can query it. OAuth lands in M4.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod db;
mod error;
mod models;
mod routes;
mod state;

use state::AppState;

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

    // Open SQLite (creates on first run).
    let conn = db::init()?;

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
    };

    let app = routes::router()
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    tracing::info!("omnidrive listening on http://{bind_addr}");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
