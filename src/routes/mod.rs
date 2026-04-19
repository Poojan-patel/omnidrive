// Route registration. Each sub-module owns its own handler(s) and we aggregate
// them into a single Router here.

use axum::{routing::get, Router};

mod home;

pub fn router() -> Router {
    Router::new().route("/", get(home::index))
}
