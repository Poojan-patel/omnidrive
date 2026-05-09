// Domain types shared across modules. Keep this file boring — no behavior,
// just data. Persistence-specific conversions (e.g., AccountStatus <-> String)
// live in the db module.

#[derive(Debug, Clone)]
pub struct Account {
    pub sub: String,
    pub email: String,
    pub name: Option<String>,
    pub picture_url: Option<String>,
    pub added_at: i64,
    pub last_refreshed_at: Option<i64>,
    pub status: AccountStatus,
}

impl Account {
    /// First character of the email, uppercased — used for the avatar
    /// placeholder when the account has no profile picture.
    pub fn initial(&self) -> String {
        initial_from_email(&self.email)
    }
}

/// Display-only projection of `Account` — the minimum fields a non-trusted
/// rendering context (the file list, future shareable views) needs to draw
/// "which account" UI. Deliberately omits `sub`, timestamps, and status so
/// those don't get carried into places that don't need them.
///
/// If `Account` ever grows new sensitive fields, they won't leak through
/// `SourceAccount` automatically — it's an explicit allowlist.
#[derive(Debug, Clone)]
pub struct SourceAccount {
    pub email: String,
    pub name: Option<String>,
    pub picture_url: Option<String>,
}

impl SourceAccount {
    pub fn initial(&self) -> String {
        initial_from_email(&self.email)
    }
}

impl From<&Account> for SourceAccount {
    fn from(a: &Account) -> Self {
        Self {
            email: a.email.clone(),
            name: a.name.clone(),
            picture_url: a.picture_url.clone(),
        }
    }
}

/// Shared helper so `Account::initial` and `SourceAccount::initial` agree.
fn initial_from_email(email: &str) -> String {
    email
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStatus {
    Active,
    NeedsReauth,
}

impl AccountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountStatus::Active => "active",
            AccountStatus::NeedsReauth => "needs_reauth",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "needs_reauth" => AccountStatus::NeedsReauth,
            _ => AccountStatus::Active, // default / unknown -> treat as active
        }
    }
}
