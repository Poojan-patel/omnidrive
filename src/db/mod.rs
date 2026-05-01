// Database bootstrap: resolve the DB path cross-platform, open a connection,
// run the schema, run idempotent migrations. Individual query functions live
// in sub-modules (accounts.rs, and more as we add tables).

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

/// Open the SQLite connection, ensure the schema is in place, and run any
/// pending idempotent migrations.
///
/// CREATE TABLE IF NOT EXISTS handles the fresh-install case. The migration
/// block below handles existing databases that pre-date a schema change —
/// each step is wrapped to swallow "duplicate column" errors so it's safe
/// to re-run on every startup.
pub fn init() -> Result<Connection> {
    let path = db_path()?;
    tracing::info!("opening database at {}", path.display());

    let conn = Connection::open(&path)
        .with_context(|| format!("failed to open database at {}", path.display()))?;

    // Recommended PRAGMAs for a single-writer SQLite app.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    conn.execute_batch(SCHEMA)?;
    run_migrations(&conn)?;

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
        status            TEXT NOT NULL DEFAULT 'active',
        refresh_token     BLOB
    );
    CREATE INDEX IF NOT EXISTS idx_accounts_status ON accounts(status);
"#;

/// Idempotent schema upgrades. Each step swallows the "duplicate column" /
/// "already exists" error so it's safe to re-run.
fn run_migrations(conn: &Connection) -> Result<()> {
    // M8: add encrypted refresh_token column to existing DBs.
    add_column_if_missing(
        conn,
        "ALTER TABLE accounts ADD COLUMN refresh_token BLOB",
    )?;
    Ok(())
}

fn add_column_if_missing(conn: &Connection, alter_sql: &str) -> Result<()> {
    match conn.execute(alter_sql, []) {
        Ok(_) => {
            tracing::info!("applied schema migration: {alter_sql}");
            Ok(())
        }
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("duplicate column") => {
            // Column already exists — nothing to do.
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("running migration: {alter_sql}")),
    }
}
