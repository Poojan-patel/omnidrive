// Google OAuth client setup. Pre-builds a reusable BasicClient (the `oauth2`
// crate's primitive) that handlers borrow to mint auth URLs and exchange codes.
//
// Why here and not in the handler: the client holds config (client_id,
// endpoints) that never change during a run, so we build it once at startup
// and stick it in AppState.

use anyhow::{Context, Result};
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl};

/// Google's OAuth 2.0 endpoints (v2 auth, v4 token).
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

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
