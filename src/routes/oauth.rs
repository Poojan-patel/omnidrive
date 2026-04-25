// OAuth handlers.
//
// /oauth/start     — mints an auth URL, stashes PKCE state, redirects to Google.
// /oauth/callback  — validates state, exchanges the code, persists the account
//                    (DB row + keyring refresh token), redirects home.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use oauth2::reqwest::async_http_client;
use oauth2::{AuthorizationCode, CsrfToken, PkceCodeChallenge, Scope, TokenResponse};
use serde::Deserialize;

use crate::db;
use crate::error::AppError;
use crate::models::{Account, AccountStatus};
use crate::oauth::{fetch_userinfo, KEYRING_SERVICE, SCOPES};
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
///
/// Happy path: validate state → exchange code → fetch userinfo → save refresh
/// token to OS keyring → upsert account row in SQLite → redirect to /.
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Result<Response, AppError> {
    // User clicked "Cancel" on the consent screen (or some other upstream
    // failure). Render a friendly page rather than 500ing.
    if let Some(err) = params.error {
        tracing::warn!("OAuth denied by user or error from Google: {err}");
        return Ok(Html(format!(
            "<!doctype html><meta charset=\"utf-8\">\
             <h1>OAuth canceled</h1>\
             <p>Reason: <code>{err}</code></p>\
             <p><a href=\"/\">Back home</a></p>"
        ))
        .into_response());
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

    let access_token = token.access_token().secret().to_string();

    // The refresh_token is what M7 will use to mint new access_tokens. If
    // Google didn't send one, the user previously consented and Google is
    // assuming we still have an old one — which we don't, since this is a
    // fresh install. `prompt=consent` in /oauth/start should prevent this,
    // but bail loudly if it ever happens so we don't silently lose access.
    let refresh_token = token
        .refresh_token()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Google didn't return a refresh_token. \
                 If you've connected this account before, revoke omnidrive at \
                 https://myaccount.google.com/permissions and try again."
            )
        })?
        .secret()
        .to_string();

    // Fetch identity from Google's userinfo endpoint. We need `sub` (Google's
    // stable user ID) before we can persist anything.
    let userinfo = fetch_userinfo(&access_token).await?;

    tracing::info!(
        sub = %userinfo.sub,
        email = %userinfo.email,
        "OAuth completed; persisting account"
    );

    // Stash the refresh_token in the OS keyring keyed by `sub`. macOS
    // Keychain / Windows Credential Manager / Linux Secret Service.
    let entry = keyring::Entry::new(KEYRING_SERVICE, &userinfo.sub)
        .map_err(|e| anyhow::anyhow!("opening keyring entry: {e}"))?;
    entry
        .set_password(&refresh_token)
        .map_err(|e| anyhow::anyhow!("writing refresh token to keyring: {e}"))?;

    // Build the Account row and upsert into SQLite.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs() as i64;
    let account = Account {
        sub: userinfo.sub,
        email: userinfo.email,
        name: userinfo.name,
        picture_url: userinfo.picture,
        added_at: now,
        last_refreshed_at: Some(now),
        status: AccountStatus::Active,
    };

    // rusqlite is sync — run the insert on the blocking pool.
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = db
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        db::accounts::upsert(&conn, &account)?;
        Ok(())
    })
    .await??;

    // Land back on the home page so the user sees their freshly-connected
    // account in the list.
    Ok(Redirect::to("/").into_response())
}
