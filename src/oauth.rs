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

use anyhow::{Context, Result};
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl};
use serde::Deserialize;

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
