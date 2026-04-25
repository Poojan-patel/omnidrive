// Account management handlers.
//
// /accounts/:sub  DELETE — remove a connected account from omnidrive.
//
// "Remove" here means: drop the SQLite row and best-effort delete the OS
// keyring entry. We *don't* call Google's revocation endpoint — the user has
// to do that themselves at https://myaccount.google.com/permissions if they
// want to fully cut omnidrive's access. The HTMX confirm dialog calls this
// out.
//
// Response body is the rendered sidebar partial (just the <ul>), which HTMX
// swaps into #sidebar-accounts on the page. No full re-render needed.

use askama::Template;
use axum::extract::{Path, State};

use crate::db;
use crate::error::AppError;
use crate::models::Account;
use crate::oauth::KEYRING_SERVICE;
use crate::state::AppState;

/// Just the sidebar account list. Same data shape as IndexTemplate so
/// `index.html` can `{% include "_accounts_list.html" %}` and we can also
/// render it standalone for HTMX swaps.
#[derive(Template)]
#[template(path = "_accounts_list.html")]
pub struct AccountsListTemplate {
    pub accounts: Vec<Account>,
}

/// DELETE /accounts/:sub
///
/// Pulls the keyring entry first (best-effort), then the DB row, then
/// re-lists and returns the partial.
pub async fn delete(
    State(state): State<AppState>,
    Path(sub): Path<String>,
) -> Result<AccountsListTemplate, AppError> {
    // Best-effort keyring delete on the blocking pool. If the entry's already
    // missing (user cleared it manually, or this is a duplicate request), we
    // log and keep going — the DB row deletion is the source of truth.
    let sub_for_keyring = sub.clone();
    tokio::task::spawn_blocking(move || {
        match keyring::Entry::new(KEYRING_SERVICE, &sub_for_keyring) {
            Ok(entry) => {
                if let Err(e) = entry.delete_password() {
                    tracing::warn!(
                        sub = %sub_for_keyring,
                        error = ?e,
                        "keyring delete failed; continuing with DB delete"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    sub = %sub_for_keyring,
                    error = ?e,
                    "couldn't open keyring entry for delete"
                );
            }
        }
    })
    .await?;

    // DB delete + re-list inside one critical section so we don't race a
    // concurrent OAuth callback.
    let db_arc = state.db.clone();
    let sub_for_db = sub.clone();
    let accounts = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Account>> {
        let conn = db_arc
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let existed = db::accounts::delete(&conn, &sub_for_db)?;
        if !existed {
            tracing::warn!(sub = %sub_for_db, "DELETE on unknown account; ignoring");
        }
        Ok(db::accounts::list(&conn)?)
    })
    .await??;

    tracing::info!(sub = %sub, "account removed");

    Ok(AccountsListTemplate { accounts })
}
