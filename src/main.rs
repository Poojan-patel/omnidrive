// omnidrive — entry point.
//
// Boots the Axum server. Initializes the SQLite connection, the OAuth client,
// the in-flight-auth map, and the access-token cache, then hands them all to
// AppState and wires up the router.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod crypto;
mod db;
mod drive;
mod error;
mod models;
mod oauth;
mod routes;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env if present — OAuth client_id/secret come from here.
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

    // Build the OAuth client from env. Fails fast if the required vars are missing.
    let oauth_client = oauth::build_client()?;

    // Load (or generate, on first run) the master key used to encrypt
    // refresh tokens stored in SQLite. Lives next to the .db file with
    // mode 0600 on Unix.
    let master_key_path = crypto::master_key_path()?;
    let master_key = crypto::MasterKey::load_or_generate(&master_key_path)?;

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        oauth_client: Arc::new(oauth_client),
        pending_auths: Arc::new(Mutex::new(HashMap::new())),
        token_cache: Arc::new(Mutex::new(HashMap::new())),
        master_key: Arc::new(master_key),
    };

    let app = routes::router()
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    tracing::info!("omnidrive listening on http://{bind_addr}");

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
