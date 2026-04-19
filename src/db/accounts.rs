// Accounts table queries. Sync functions — callers wrap these in
// `tokio::task::spawn_blocking` so the async runtime isn't blocked on SQLite.

use rusqlite::{Connection, Result};

use crate::models::{Account, AccountStatus};

/// Returns all connected accounts, newest first.
pub fn list(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare(
        "SELECT sub, email, name, picture_url, added_at, last_refreshed_at, status
         FROM accounts
         ORDER BY added_at DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let status_str: String = row.get(6)?;
        Ok(Account {
            sub: row.get(0)?,
            email: row.get(1)?,
            name: row.get(2)?,
            picture_url: row.get(3)?,
            added_at: row.get(4)?,
            last_refreshed_at: row.get(5)?,
            status: AccountStatus::from_db(&status_str),
        })
    })?;

    rows.collect()
}
