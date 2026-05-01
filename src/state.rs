// App-wide shared state. Injected into every handler via axum's State extractor.
//
// Fields:
// - `db`: SQLite connection behind Arc<Mutex<...>>. Rusqlite is sync, so
//   handlers take the lock inside `spawn_blocking`.
// - `oauth_client`: pre-built oauth2 client; handlers use it to mint auth URLs
//   and exchange codes.
// - `pending_auths`: in-flight OAuth attempts keyed by the `state` parameter.
//   We stash the PKCE verifier here between `/oauth/start` and `/oauth/callback`
//   so the callback can complete the exchange. For a single-user local app an
//   in-memory map is plenty; stale entries are evicted after 10 minutes.
// - `token_cache`: in-memory access-token cache keyed by Google `sub`. Access
//   tokens last ~1 hour; we cache them to avoid hitting Google's token
//   endpoint on every Drive API call. The refresh primitive in `oauth.rs`
//   reads/writes this map.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use oauth2::basic::BasicClient;
use oauth2::PkceCodeVerifier;
use rusqlite::Connection;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub oauth_client: Arc<BasicClient>,
    pub pending_auths: Arc<Mutex<HashMap<String, PendingAuth>>>,
    pub token_cache: Arc<Mutex<HashMap<String, CachedToken>>>,
}

pub struct PendingAuth {
    pub pkce_verifier: PkceCodeVerifier,
    pub created_at: Instant,
}

#[derive(Clone)]
pub struct CachedToken {
    pub access_token: String,
    pub expires_at: Instant,
}
