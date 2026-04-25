// GET / — the home page. Queries the connected-accounts list from SQLite and
// hands it to the template, which renders the Drive-style sidebar + main pane
// shell. The same `accounts` field is forwarded into the _accounts_list.html
// partial via `{% include %}`.

use askama::Template;
use axum::extract::State;

use crate::db;
use crate::error::AppError;
use crate::models::Account;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub accounts: Vec<Account>,
}

pub async fn index(State(state): State<AppState>) -> Result<IndexTemplate, AppError> {
    // rusqlite is sync, so run the query on the blocking thread pool.
    let accounts = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Account>> {
        let conn = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        Ok(db::accounts::list(&conn)?)
    })
    .await??; // first ? for JoinError, second for the inner Result

    Ok(IndexTemplate { accounts })
}
