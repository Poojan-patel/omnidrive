// Route registration. Each sub-module owns its own handler(s) and we aggregate
// them into a single Router here.

use axum::{routing::get, Router};

use crate::state::AppState;

mod home;
mod oauth;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home::index))
        .route("/oauth/start", get(oauth::start))
        .route("/oauth/callback", get(oauth::callback))
}
