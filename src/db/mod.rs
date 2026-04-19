// Database bootstrap: resolve the DB path cross-platform, open a connection,
// run the schema. Individual query functions live in sub-modules (accounts.rs,
// and more as we add tables).

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use rusqlite::Connection;

pub mod accounts;

/// Resolve the SQLite file location using platform conventions:
/// - Linux:   ~/.local/share/omnidrive/omnidrive.db
/// - macOS:   ~/Library/Application Support/com.ppatel.omnidrive/omnidrive.db
/// - Windows: %APPDATA%\ppatel\omnidrive\data\omnidrive.db
pub fn db_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "ppatel", "omnidrive")
        .context("could not determine project data directory")?;
    let data_dir = dirs.data_dir();
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create data dir at {}", data_dir.display()))?;
    Ok(data_dir.join("omnidrive.db"))
}

/// Open the SQLite connection and ensure the schema is in place.
/// Safe to call on every startup — CREATE TABLE IF NOT EXISTS is idempotent.
pub fn init() -> Result<Connection> {
    let path = db_path()?;
    tracing::info!("opening database at {}", path.display());

    let conn = Connection::open(&path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;

    // Recommended PRAGMAs for a single-writer SQLite app.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS accounts (
        sub               TEXT PRIMARY KEY,
        email             TEXT NOT NULL,
        name              TEXT,
        picture_url       TEXT,
        added_at          INTEGER NOT NULL,
        last_refreshed_at INTEGER,
        status            TEXT NOT NULL DEFAULT 'active'
    );
    CREATE INDEX IF NOT EXISTS idx_accounts_status ON accounts(status);
"#;
