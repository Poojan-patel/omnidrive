// Accounts table queries. Sync functions — callers wrap these in
// `tokio::task::spawn_blocking` so the async runtime isn't blocked on SQLite.

use rusqlite::{params, Connection, Result};

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

/// Remove an account row by `sub`. Returns whether a row actually existed.
/// Caller is responsible for cleaning up the matching keyring entry.
pub fn delete(conn: &Connection, sub: &str) -> Result<bool> {
    let rows = conn.execute("DELETE FROM accounts WHERE sub = ?1", params![sub])?;
    Ok(rows > 0)
}

/// Update an account's status (e.g., flip to NeedsReauth when refresh fails).
/// Also bumps last_refreshed_at to the current epoch second when the status
/// becomes Active again, so the UI can show a fresh timestamp.
pub fn set_status(conn: &Connection, sub: &str, status: AccountStatus) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE accounts
            SET status = ?1,
                last_refreshed_at = CASE WHEN ?1 = 'active' THEN ?2 ELSE last_refreshed_at END
          WHERE sub = ?3",
        params![status.as_str(), now, sub],
    )?;
    Ok(())
}

/// Insert a new account or update the metadata of an existing one (matched by
/// `sub`). Crucially, `added_at` is *not* overwritten on conflict — we want
/// to preserve the original "first connected" timestamp even if the user
/// reconnects later (e.g., after a refresh-token expiry).
pub fn upsert(conn: &Connection, account: &Account) -> Result<()> {
    conn.execute(
        "INSERT INTO accounts
            (sub, email, name, picture_url, added_at, last_refreshed_at, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(sub) DO UPDATE SET
             email             = excluded.email,
             name              = excluded.name,
             picture_url       = excluded.picture_url,
             last_refreshed_at = excluded.last_refreshed_at,
             status            = excluded.status",
        params![
            account.sub,
            account.email,
            account.name,
            account.picture_url,
            account.added_at,
            account.last_refreshed_at,
            account.status.as_str(),
        ],
    )?;
    Ok(())
}
