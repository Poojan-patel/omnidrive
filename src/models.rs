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
        self.email
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
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
