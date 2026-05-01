// Route registration. Each sub-module owns its own handler(s) and we aggregate
// them into a single Router here.

use axum::{
    routing::{delete, get},
    Router,
};

use crate::state::AppState;

mod accounts;
mod home;
mod oauth;
mod search;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home::index))
        .route("/search", get(search::search))
        .route("/oauth/start", get(oauth::start))
        .route("/oauth/callback", get(oauth::callback))
        .route("/accounts/:sub", delete(accounts::delete))
}
