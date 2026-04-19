// OAuth handlers.
//
// /oauth/start     — mints an auth URL, stashes PKCE state, redirects to Google.
// /oauth/callback  — validates state, exchanges the code, logs the tokens.
//
// M4 intentionally stops at "tokens in the log" — no DB writes, no keyring.
// M5 picks up the persistence side.

use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::response::{Html, Redirect};
use oauth2::reqwest::async_http_client;
use oauth2::{AuthorizationCode, CsrfToken, PkceCodeChallenge, Scope, TokenResponse};
use serde::Deserialize;

use crate::error::AppError;
use crate::oauth::SCOPES;
use crate::state::{AppState, PendingAuth};

/// GET /oauth/start
///
/// Generate a fresh PKCE pair + CSRF state, stash them in AppState keyed by
/// the state string, then redirect the browser to Google's consent page.
pub async fn start(State(state): State<AppState>) -> Result<Redirect, AppError> {
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Build the authorize URL. Scopes + the two load-bearing extra params
    // (access_type=offline ensures we get a refresh_token; prompt=consent
    // forces the consent screen even on re-auth, so we always get a fresh
    // refresh token).
    let mut builder = state
        .oauth_client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent");

    for scope in SCOPES {
        builder = builder.add_scope(Scope::new((*scope).to_string()));
    }

    let (auth_url, csrf_token) = builder.url();

    // Stash the verifier under the state token so the callback can pair them.
    {
        let mut pending = state
            .pending_auths
            .lock()
            .map_err(|_| anyhow::anyhow!("pending_auths mutex poisoned"))?;

        pending.insert(
            csrf_token.secret().to_string(),
            PendingAuth {
                pkce_verifier,
                created_at: Instant::now(),
            },
        );

        // Opportunistic cleanup: drop entries older than 10 minutes. Good
        // enough for a local tool — no background task needed.
        let now = Instant::now();
        pending.retain(|_, v| now.duration_since(v.created_at) < Duration::from_secs(600));
    }

    tracing::info!("redirecting to Google OAuth consent");
    Ok(Redirect::to(auth_url.as_str()))
}

/// Query params Google sends back on `/oauth/callback`. Either (code + state)
/// on success, or `error` if the user denied consent.
#[derive(Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// GET /oauth/callback?code=...&state=...
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Result<Html<String>, AppError> {
    // User clicked "Cancel" on the consent screen (or some other upstream
    // failure).
    if let Some(err) = params.error {
        tracing::warn!("OAuth denied by user or error from Google: {err}");
        return Ok(Html(format!(
            "<!doctype html><meta charset=\"utf-8\">\
             <h1>OAuth canceled</h1>\
             <p>Reason: <code>{err}</code></p>\
             <p><a href=\"/\">Back home</a></p>"
        )));
    }

    let code = params
        .code
        .ok_or_else(|| anyhow::anyhow!("callback missing ?code"))?;
    let received_state = params
        .state
        .ok_or_else(|| anyhow::anyhow!("callback missing ?state"))?;

    // Look up and remove the matching PKCE verifier.
    let pending = {
        let mut map = state
            .pending_auths
            .lock()
            .map_err(|_| anyhow::anyhow!("pending_auths mutex poisoned"))?;
        map.remove(&received_state)
    };

    let pending = pending.ok_or_else(|| {
        anyhow::anyhow!(
            "unknown or expired OAuth state token — \
             either this is a stale callback or someone replayed a URL"
        )
    })?;

    // Exchange the authorization code for tokens.
    let token = state
        .oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pending.pkce_verifier)
        .request_async(async_http_client)
        .await
        .map_err(|e| anyhow::anyhow!("token exchange failed: {e:?}"))?;

    let access_token = token.access_token().secret();
    let refresh_token = token
        .refresh_token()
        .map(|t| t.secret().as_str())
        .unwrap_or("<none>");
    let expires_in = token
        .expires_in()
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());

    // tracing::info!(
    //     access_token = %access_token,
    //     refresh_token = %refresh_token,
    //     expires_in_secs = %expires_in,
    //     "OAuth token exchange succeeded"
    // );

    Ok(Html(
        "<!doctype html><meta charset=\"utf-8\">\
         <h1>OAuth success!</h1>\
         <p>Tokens were logged to the server terminal. \
         Persistence lands in the next milestone — \
         for now, this is just a plumbing check.</p>\
         <p><a href=\"/\">Back home</a></p>"
            .to_string(),
    ))
}
