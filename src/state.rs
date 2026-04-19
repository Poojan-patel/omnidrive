// App-wide shared state. Injected into every handler via axum's State extractor.
//
// For M3 this holds only the SQLite connection behind Arc<Mutex<...>>. M4
// extends it with the OAuth client and in-flight-auth tracking.
//
// Rusqlite is sync, so handlers take the lock inside `spawn_blocking`.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
}
