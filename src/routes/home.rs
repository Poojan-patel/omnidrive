// GET / — the home page.
//
// In M7 this got real: rather than just listing accounts, the home page now
// fans out to every connected drive in parallel, lists the files at the root
// of each, k-way-merges them by modifiedTime, and renders the unified table.
//
// The same template (with a `mode` flag and a query echo) is reused for the
// search route — see routes/search.rs.

use askama::Template;
use axum::extract::State;
use tokio::task::JoinSet;

use crate::db;
use crate::drive::{self, FileWithSource};
use crate::error::AppError;
use crate::models::{Account, SourceAccount};
use crate::oauth;
use crate::state::AppState;

/// Drives the index.html template. `query` is None on the home page; the
/// search route renders the same template with Some(query).
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub accounts: Vec<Account>,
    pub files: Vec<FileWithSource>,
    /// Accounts whose Drive call failed (network error, refresh failed, etc.).
    /// Surfaced as a small banner above the file list.
    pub failed_accounts: Vec<String>,
    /// None on home (`/`); Some(q) when rendering search results.
    pub query: Option<String>,
}

pub async fn index(State(state): State<AppState>) -> Result<IndexTemplate, AppError> {
    let accounts = list_accounts(&state).await?;

    // No accounts = no Drive calls. Render the empty-state template.
    if accounts.is_empty() {
        return Ok(IndexTemplate {
            accounts,
            files: vec![],
            failed_accounts: vec![],
            query: None,
        });
    }

    let (files, failed_accounts) = fan_out(&state, &accounts, FetchMode::Root).await;

    Ok(IndexTemplate {
        accounts,
        files,
        failed_accounts,
        query: None,
    })
}

/// Pull the accounts list from the DB. Shared by index and search.
pub(crate) async fn list_accounts(state: &AppState) -> Result<Vec<Account>, AppError> {
    let db_arc = state.db.clone();
    let accounts = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Account>> {
        let conn = db_arc
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        Ok(db::accounts::list(&conn)?)
    })
    .await??;
    Ok(accounts)
}

/// What to fetch from each account.
pub(crate) enum FetchMode<'a> {
    Root,
    Search(&'a str),
}

/// Fan out to every account in parallel, collect successful streams, and
/// k-way-merge them. Returns (merged_files, failed_account_emails).
pub(crate) async fn fan_out(
    state: &AppState,
    accounts: &[Account],
    mode: FetchMode<'_>,
) -> (Vec<FileWithSource>, Vec<String>) {
    let query_owned: Option<String> = match mode {
        FetchMode::Root => None,
        FetchMode::Search(q) => Some(q.to_string()),
    };

    // Spawn one task per account. We carry the whole Account into the result
    // tuple so a successful task can attach it to each FileWithSource (for
    // avatar/name rendering in the Location column), and a failed task can
    // still report which account failed by email.
    let mut set: JoinSet<(Account, anyhow::Result<Vec<drive::DriveFile>>)> = JoinSet::new();
    for account in accounts {
        let state_clone = state.clone();
        let account_clone = account.clone();
        let sub = account.sub.clone();
        let query_clone = query_owned.clone();
        set.spawn(async move {
            let result = async {
                let token = oauth::get_access_token(&state_clone, &sub).await?;
                match query_clone {
                    None => drive::list_root_files(&token).await,
                    Some(q) => drive::search_files(&token, &q).await,
                }
            }
            .await;
            (account_clone, result)
        });
    }

    let mut per_account: Vec<Vec<FileWithSource>> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((account, Ok(files))) => {
                // Project Account -> SourceAccount once per task; each file
                // for this account just clones the small projection.
                let source: SourceAccount = (&account).into();
                let with_source: Vec<FileWithSource> = files
                    .into_iter()
                    .map(|f| FileWithSource {
                        file: f,
                        account: source.clone(),
                    })
                    .collect();
                per_account.push(with_source);
            }
            Ok((account, Err(e))) => {
                tracing::warn!(email = %account.email, error = ?e, "Drive fetch failed for account");
                failed.push(account.email);
            }
            Err(join_err) => {
                tracing::error!(error = ?join_err, "Drive fetch task panicked");
            }
        }
    }

    let merged = drive::k_way_merge_by_mtime(per_account);
    (merged, failed)
}
