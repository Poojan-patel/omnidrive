# omnidrive

> Unified search across multiple Google Drive accounts.

A local web app for people who've scattered their files across many Google accounts and can't remember which Drive holds what. Modeled on Gmail's unified-inbox search UX, but for Drive.

**Status:** working personal-use prototype. Search across multiple drives, account management, and encrypted token storage are all functional. Not packaged for distribution.

## Why

Many people maintain several Gmail accounts, partly for the free 15GB of Drive storage that comes with each one. Finding "that file I saved a year ago" then becomes a tour through every account's Drive UI. omnidrive lets you connect all your accounts once, then search across them from a single search bar.

## What it does

- **Connect any number of Google accounts** via the standard OAuth consent flow. Each one is a small avatar in the left sidebar.
- **Home page lists files at the root of every connected drive**, k-way-merged by modified time so the most-recent files surface across all accounts.
- **Search the filenames across every drive in parallel** from a single search box. Multi-word queries become AND-ed `name contains` clauses; results merge sorted by modified time.
- **Click a file** to open it directly in Google Drive in a new tab.
- **Click × on an account** to remove it locally (HTMX swap, no full page reload). The OAuth grant at Google's side is preserved — revoke at <https://myaccount.google.com/permissions> if you want to fully cut access.

## How it works

- **Local-only.** omnidrive runs entirely on your machine. No third-party server, no cloud component, no analytics.
- **Bring-your-own GCP credentials.** You create a Google Cloud project + OAuth client and paste the credentials into `.env`. omnidrive never touches anyone else's project, so your data stays in your trust boundary.
- **Refresh tokens encrypted at rest.** Stored as AES-256-GCM ciphertext in the local SQLite file, with a 32-byte master key in a sibling `master.key` file (mode `0600`). The encryption key can also be supplied via the `OMNIDRIVE_MASTER_KEY` env var (base64-encoded) if you'd rather manage it externally.
- **Token refresh is automatic.** Access tokens are cached in memory with a 60s safety margin and refreshed against Google's token endpoint when they expire. Revoked accounts get flagged `NeedsReauth` and surface a "couldn't load" banner; reconnecting via "Add account" reactivates them.

## Setup

### 1. Google Cloud project

1. Create a new project at [console.cloud.google.com](https://console.cloud.google.com).
2. Enable the **Google Drive API** in *APIs & Services → Library*.
3. Configure the **OAuth consent screen**:
   - User Type: **External**
   - Add the scopes `.../auth/drive.metadata.readonly`, `openid`, `.../auth/userinfo.email`, `.../auth/userinfo.profile`
   - Add the Gmail addresses you want to connect to **Test users** (up to 100)
   - Leave publishing status as **Testing**
4. Create an **OAuth 2.0 Client ID**:
   - Application type: **Web application**
   - Authorized redirect URI: `http://127.0.0.1:8765/oauth/callback`
5. Note the Client ID and Client Secret.

> **Note on the 7-day expiry:** because the app stays in Testing mode, refresh tokens Google issues expire after **7 days**. omnidrive surfaces a "couldn't load files" banner when this happens — click **Add account** with the same Google account to reconnect. This is Google's policy for unverified test apps, not an omnidrive limitation. Move the consent screen to "Production" if you want longer-lived tokens.

### 2. Run

```bash
cp .env.example .env
# edit .env and paste your GOOGLE_CLIENT_ID + GOOGLE_CLIENT_SECRET
cargo run
```

Then open <http://127.0.0.1:8765>.

First run will:
- create the SQLite database at the platform-appropriate data directory (`~/Library/Application Support/com.ppatel.omnidrive/` on macOS, `~/.local/share/omnidrive/` on Linux, `%APPDATA%\ppatel\omnidrive\data\` on Windows)
- generate `master.key` next to it with mode `0600`
- log both paths so you can find them

### 3. Optional: override the master key

To supply the encryption key from the environment instead of the file (useful for containerized deploys or if you'd rather store the key in a password manager):

```bash
OMNIDRIVE_MASTER_KEY=$(head -c 32 /dev/urandom | base64) cargo run
```

If both the env var and the file exist, the env var wins.

## Tech stack

- **Backend:** Rust + [Axum](https://github.com/tokio-rs/axum) + [Tokio](https://tokio.rs)
- **UI:** server-rendered [Askama](https://github.com/djc/askama) templates with [HTMX](https://htmx.org) for interactivity (no JS build step). Wordmark in [Space Grotesk](https://fonts.google.com/specimen/Space+Grotesk).
- **Storage:** [rusqlite](https://github.com/rusqlite/rusqlite) (bundled SQLite) for account metadata + encrypted refresh tokens, with a sibling `master.key` file holding the AES-256-GCM key.
- **OAuth:** [`oauth2`](https://github.com/ramosbugs/oauth2-rs) crate.
- **Crypto:** [`aes-gcm`](https://github.com/RustCrypto/AEADs/tree/master/aes-gcm) (RustCrypto), `rand` from the OS CSPRNG, `base64` for env-var encoding.

## Project layout

```
src/
  main.rs              entry point — boot the Axum server
  state.rs             AppState shared across handlers
  models.rs            Account + SourceAccount domain types
  error.rs             axum + anyhow IntoResponse glue
  crypto.rs            AES-256-GCM master key + encrypt/decrypt
  oauth.rs             Google OAuth client + refresh primitive + userinfo
  drive.rs             Drive API client + k-way merge of result streams
  db/
    mod.rs             schema bootstrap + migrations
    accounts.rs        accounts table queries (list, upsert, delete, ...)
  routes/
    mod.rs             route registration
    home.rs            GET /  — fan-out to each drive, render unified table
    search.rs          GET /search?q=...  — same shape, with the q param
    oauth.rs           /oauth/start + /oauth/callback
    accounts.rs        DELETE /accounts/:sub
templates/
  base.html            shell + design tokens + CSS
  index.html           home + search results (same template)
  _accounts_list.html  sidebar partial (HTMX swap target)
  _file_list.html      file table partial
```

## Development

### Pre-commit hooks (secret scanning + hygiene)

This repo ships a [pre-commit](https://pre-commit.com) config that runs [gitleaks](https://github.com/gitleaks/gitleaks) and a few hygiene checks on every commit. Set it up once per clone:

```bash
brew install pre-commit       # or: pip install pre-commit
pre-commit install            # wires the git hook
```

That's it — every `git commit` will now run gitleaks against the staged diff before the commit lands. If gitleaks finds something that looks like a credential, the commit is rejected and you see the offending line + the rule that fired.

To scan the entire repo manually (handy after you first install, or after editing `.gitleaks.toml`):

```bash
pre-commit run --all-files
```

Custom config lives in `.gitleaks.toml`. The allowlist there carves out the placeholder strings we ship in `.env.example` and the docs (e.g. `paste-your-client-secret-here`) so they don't generate false positives. Two project-specific custom rules also catch:

- `OMNIDRIVE_MASTER_KEY=<32-byte base64>` accidentally committed
- Google refresh tokens (the `1//0…` shape)

If you ever need to commit something gitleaks doesn't like and you've reviewed it, `git commit --no-verify` skips the hook for one commit. Use sparingly.

## Threat model (honest version)

omnidrive's encryption defends against **accidental** exposure of the SQLite file — backup tarballs, screenshots, an accidental `git add data/`, etc. The encrypted blob alone is useless without `master.key`.

It does **not** defend against an attacker who has read access to your whole home directory, since they'd have both the database and the key. That's a problem for OS-level disk encryption (FileVault on macOS, dm-crypt on Linux, BitLocker on Windows), which is your responsibility, not omnidrive's.

## Possible follow-ups

Things that would be natural next milestones if/when this is picked up again:

- Periodic background token-refresh sweep (detect revocation/7-day-expiry without waiting for the next Drive call)
- Auto-heal on Drive 401 (invalidate cached access_token + retry once)
- Dark mode (CSS custom properties already in place; `prefers-color-scheme` would be a small delta)
- Filter by file type, owner, modified date
- Full-text search (requires upgrading the OAuth scope from `drive.metadata.readonly` to `drive.readonly`)
- Pagination (k-way merge with cursor-encoded per-account state)
- File preview on click (instead of opening in Drive's UI)

## License

MIT
