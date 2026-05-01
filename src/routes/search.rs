// GET /search?q=...
//
// Reuses the same IndexTemplate as the home page — the only differences are
// (a) we call drive::search_files instead of drive::list_root_files, and
// (b) we set the `query` field so the template can echo it.
//
// An empty/whitespace query just redirects to /. We don't render an empty
// search results page — feels worse than the home view.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use super::home::{fan_out, list_accounts, FetchMode, IndexTemplate};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String,
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Response, AppError> {
    let trimmed = params.q.trim();
    if trimmed.is_empty() {
        return Ok(Redirect::to("/").into_response());
    }

    let accounts = list_accounts(&state).await?;

    if accounts.is_empty() {
        // No accounts to search. Render the index empty-state with the query
        // echoed so the searchbar keeps its content.
        return Ok(IndexTemplate {
            accounts,
            files: vec![],
            failed_accounts: vec![],
            query: Some(trimmed.to_string()),
        }
        .into_response());
    }

    let (files, failed_accounts) =
        fan_out(&state, &accounts, FetchMode::Search(trimmed)).await;

    Ok(IndexTemplate {
        accounts,
        files,
        failed_accounts,
        query: Some(trimmed.to_string()),
    }
    .into_response())
}
