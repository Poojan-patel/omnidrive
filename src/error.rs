// AppError — the canonical axum + anyhow glue.
//
// axum handlers need to return types that implement IntoResponse. anyhow::Error
// doesn't, so we wrap it. The blanket From<E: Into<anyhow::Error>> impl means
// any Result<T, E> inside a handler can use `?` to bubble up to a 500.
//
// When a real error reaches IntoResponse we log the full debug chain and
// return a generic 500 to the user (don't leak internals to the browser).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("handler error: {:#}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error. Check logs for details.",
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
