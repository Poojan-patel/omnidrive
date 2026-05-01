// Account management handlers.
//
// /accounts/:sub  DELETE — remove a connected account from omnidrive.
//
// "Remove" here means: drop the SQLite row, which takes the encrypted
// refresh_token blob with it. We *don't* call Google's revocation endpoint
// — the user has to do that themselves at
// https://myaccount.google.com/permissions if they want to fully cut
// omnidrive's access. The HTMX confirm dialog calls this out.
//
// Response body is the rendered sidebar partial (just the <ul>), which HTMX
// swaps into #sidebar-accounts on the page. No full re-render needed.

use askama::Template;
use axum::extract::{Path, State};

use crate::db;
use crate::error::AppError;
use crate::models::Account;
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
/// Deletes the row (which takes the encrypted refresh_token with it),
/// evicts the cached access_token, and re-lists for the HTMX swap.
/// Idempotent — deleting an already-removed account just renders an
/// unchanged list.
pub async fn delete(
    State(state): State<AppState>,
    Path(sub): Path<String>,
) -> Result<AccountsListTemplate, AppError> {
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

    // Evict the cached access_token for this account. Without this, a
    // re-add of the same Google account would inherit the stale cached
    // token until natural expiry — see also the matching cache-update in
    // routes::oauth::callback for the symmetric "reconnect after revoke"
    // case.
    {
        let mut cache = state
            .token_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("token_cache mutex poisoned"))?;
        cache.remove(&sub);
    }

    tracing::info!(sub = %sub, "account removed");

    Ok(AccountsListTemplate { accounts })
}
