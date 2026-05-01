// Google OAuth client setup. Pre-builds a reusable BasicClient (the `oauth2`
// crate's primitive) that handlers borrow to mint auth URLs and exchange codes.
//
// Why here and not in the handler: the client holds config (client_id,
// endpoints) that never change during a run, so we build it once at startup
// and stick it in AppState.
//
// Also home to the userinfo fetch helper. After the token exchange we call
// Google's userinfo endpoint to learn *which* account just connected (sub,
// email, name, picture) before we persist anything.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{AuthUrl, ClientId, ClientSecret, RedirectUrl, RefreshToken, TokenResponse, TokenUrl};
use serde::Deserialize;

use crate::db;
use crate::models::AccountStatus;
use crate::state::{AppState, CachedToken};

/// Google's OAuth 2.0 endpoints (v2 auth, v4 token).
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google's OpenID Connect userinfo endpoint. Returns sub/email/name/picture
/// for the account whose access_token is presented as a Bearer credential.
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// Service name used for OS keyring entries. Refresh tokens are stored as
/// (service=KEYRING_SERVICE, username=sub, password=refresh_token).
pub const KEYRING_SERVICE: &str = "omnidrive";

/// Scopes we'll request on consent.
///
/// `drive.metadata.readonly` is enough for filename + metadata search. We'll
/// upgrade to `drive.readonly` later if we add file preview / download.
/// `openid`, `userinfo.email`, `userinfo.profile` are what populate the
/// id_token so we can identify which account just connected.
pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/drive.metadata.readonly",
    "openid",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
];

pub fn build_client() -> Result<BasicClient> {
    let client_id = std::env::var("GOOGLE_CLIENT_ID")
        .context("GOOGLE_CLIENT_ID not set — copy .env.example to .env and fill it in")?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")
        .context("GOOGLE_CLIENT_SECRET not set — copy .env.example to .env and fill it in")?;
    let redirect_uri = std::env::var("OAUTH_REDIRECT_URI")
        .unwrap_or_else(|_| "http://127.0.0.1:8765/oauth/callback".to_string());

    let client = BasicClient::new(
        ClientId::new(client_id),
        Some(ClientSecret::new(client_secret)),
        AuthUrl::new(GOOGLE_AUTH_URL.to_string())?,
        Some(TokenUrl::new(GOOGLE_TOKEN_URL.to_string())?),
    )
    .set_redirect_uri(RedirectUrl::new(redirect_uri)?);

    Ok(client)
}

/// Subset of Google's UserInfo response we care about.
///
/// `sub` is Google's stable, opaque user identifier — it never changes even
/// if the user changes their email, so it's our primary key. The other
/// fields are best-effort metadata for display.
#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

/// Call Google's OpenID userinfo endpoint with the freshly-minted access_token
/// to find out who just authenticated.
pub async fn fetch_userinfo(access_token: &str) -> Result<UserInfo> {
    let resp = reqwest::Client::new()
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .context("calling Google userinfo endpoint")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("userinfo endpoint returned {status}: {body}");
    }

    resp.json::<UserInfo>()
        .await
        .context("decoding userinfo response")
}

/// Refresh-with-cache primitive. Returns a valid access_token for `sub`,
/// refreshing through Google's token endpoint if the cached one is expired
/// (or about to expire — we use a 60s safety margin so a request that takes
/// ~30s doesn't strand on a token that expires mid-flight).
///
/// On `invalid_grant` from Google (refresh_token revoked or 7-day-expired in
/// Test mode), the account is marked NeedsReauth in the DB and the keyring
/// entry is left in place for inspection. The user reconnects via the
/// normal /oauth/start flow; that upserts the row + new refresh_token and
/// flips status back to Active.
pub async fn get_access_token(state: &AppState, sub: &str) -> Result<String> {
    // Fast path: cache hit with comfy expiry margin.
    {
        let cache = state
            .token_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("token_cache mutex poisoned"))?;
        if let Some(cached) = cache.get(sub) {
            if cached.expires_at > Instant::now() + Duration::from_secs(60) {
                return Ok(cached.access_token.clone());
            }
        }
    } // drop the lock — we don't want to hold it across HTTP calls.

    // Read the refresh_token from the OS keyring. Sync — spawn_blocking.
    let sub_for_keyring = sub.to_string();
    let refresh_token = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &sub_for_keyring)
            .map_err(|e| anyhow::anyhow!("opening keyring entry: {e}"))?;
        entry
            .get_password()
            .map_err(|e| anyhow::anyhow!("reading refresh token from keyring: {e}"))
    })
    .await??;

    // Hit Google's token endpoint with the refresh_token.
    let exchange_result = state
        .oauth_client
        .exchange_refresh_token(&RefreshToken::new(refresh_token))
        .request_async(async_http_client)
        .await;

    let token = match exchange_result {
        Ok(t) => t,
        Err(e) => {
            // The oauth2 crate's error type doesn't expose `error` cleanly
            // without pattern-matching on its enum, so we fall back to the
            // rendered debug string. `invalid_grant` is the canonical signal
            // that the refresh token is dead.
            let err_str = format!("{e:?}");
            if err_str.contains("invalid_grant") {
                tracing::warn!(sub = %sub, "refresh token rejected — marking NeedsReauth");
                let db_arc = state.db.clone();
                let sub_for_db = sub.to_string();
                let _ = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let conn = db_arc
                        .lock()
                        .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
                    db::accounts::set_status(&conn, &sub_for_db, AccountStatus::NeedsReauth)?;
                    Ok(())
                })
                .await;
            }
            return Err(anyhow::anyhow!("token refresh failed: {err_str}"));
        }
    };

    let access_token = token.access_token().secret().to_string();
    let expires_in = token.expires_in().unwrap_or(Duration::from_secs(3600));

    // Update cache.
    {
        let mut cache = state
            .token_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("token_cache mutex poisoned"))?;
        cache.insert(
            sub.to_string(),
            CachedToken {
                access_token: access_token.clone(),
                expires_at: Instant::now() + expires_in,
            },
        );
    }

    // Touch the DB so last_refreshed_at advances and (if the row was
    // NeedsReauth from a prior failure) flip it back to Active.
    let db_arc = state.db.clone();
    let sub_for_db = sub.to_string();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let conn = db_arc
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        db::accounts::set_status(&conn, &sub_for_db, AccountStatus::Active)?;
        Ok(())
    })
    .await??;

    Ok(access_token)
}
