# omnidrive

> Unified search across multiple Google Drive accounts.

A local web app for people who've scattered their files across many Google accounts and can't remember which Drive holds what. Modeled on Gmail's unified-inbox search UX, but for Drive.

**Status:** under active development. Not production-ready.

## Why

Many people maintain several Gmail accounts, partly for the free 15GB of Drive storage that comes with each one. Finding "that file I saved a year ago" then becomes a tour through every account's Drive UI. omnidrive lets you connect all your accounts once, then search across them from a single search bar.

## How it works

- You bring your own Google Cloud project and OAuth credentials (see Setup below).
- omnidrive runs entirely on your machine — no third-party server, no cloud component.
- Refresh tokens are stored encrypted (AES-256-GCM, per-row random nonce) inside the local SQLite file. The encryption key lives in a sibling `master.key` file, mode `0600`.
- Searches fan out to each connected account's Drive API in parallel and merge the results.

## Setup

### 1. Google Cloud project

1. Create a new project at [console.cloud.google.com](https://console.cloud.google.com).
2. Enable the **Google Drive API** in *APIs & Services → Library*.
3. Configure the **OAuth consent screen**:
   - User Type: **External**
   - Add the scope `.../auth/drive.metadata.readonly` (plus `openid`, `userinfo.email`, `userinfo.profile`)
   - Add your Gmail addresses to **Test users** (you can add up to 100)
   - Leave publishing status as **Testing**
4. Create an **OAuth 2.0 Client ID**:
   - Application type: **Web application**
   - Authorized redirect URI: `http://127.0.0.1:8765/oauth/callback`
5. Download the JSON, or note the Client ID and Client Secret.

> **Note:** because the app stays in Testing mode, refresh tokens issued by Google expire after **7 days**. omnidrive surfaces a "reconnect" prompt when this happens — you'll re-auth roughly weekly. This is a Google policy for unverified test apps, not an omnidrive limitation.

### 2. Run

```bash
cp .env.example .env
# edit .env and paste your GOOGLE_CLIENT_ID + GOOGLE_CLIENT_SECRET
cargo run
```

Then open <http://127.0.0.1:8765>.

## Tech stack

- **Backend:** Rust + [Axum](https://github.com/tokio-rs/axum) + [tokio](https://tokio.rs)
- **UI:** server-rendered [Askama](https://github.com/djc/askama) templates with [HTMX](https://htmx.org) for interactivity (no JS build step)
- **Storage:** [rusqlite](https://github.com/rusqlite/rusqlite) (bundled SQLite) for account metadata + encrypted refresh tokens, with a sibling `master.key` file holding the AES-256-GCM key
- **OAuth:** [`oauth2`](https://github.com/ramosbugs/oauth2-rs) crate

## License

MIT
