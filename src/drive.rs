// Drive API client. Just the two operations we need today:
//   - list_root_files: list files at the root of a connected drive
//   - search_files:    name-substring search across a connected drive
//
// Both return a Vec<DriveFile> sorted by modifiedTime desc (because we ask
// Drive to do the sort server-side via orderBy). The route handlers fan out
// to all connected accounts in parallel and then k_way_merge_by_mtime() the
// per-account streams into a single globally-sorted list.
//
// Scope: drive.metadata.readonly. We can match on filename, mimeType, and
// other metadata, but NOT on file content (that's drive.readonly territory).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use anyhow::{Context, Result};
use serde::Deserialize;

const DRIVE_FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";

/// We ask Drive for these specific fields on every list/search call. Keeping
/// this tight keeps response payloads small and parsing fast.
const FIELDS: &str = "files(id,name,mimeType,modifiedTime,webViewLink,\
                      lastModifyingUser(displayName,emailAddress))";

/// Per-account fetch cap. With no pagination, this is the absolute max
/// we'll show from any one drive in a single render. 50 is generous for
/// "what's at the root of my drive" and "name contains some-word".
const PAGE_SIZE: u32 = 50;

/// One file as Drive sends it back. All optional fields may be missing on
/// real data — particularly `lastModifyingUser` for files the API user owns
/// and last edited via the API, where Drive sometimes omits the user block.
#[derive(Debug, Deserialize, Clone)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "modifiedTime", default)]
    pub modified_time: Option<String>,
    #[serde(rename = "webViewLink", default)]
    pub web_view_link: Option<String>,
    #[serde(rename = "lastModifyingUser", default)]
    pub last_modifying_user: Option<DriveUser>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DriveUser {
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "emailAddress", default)]
    pub email_address: Option<String>,
}

impl DriveFile {
    pub fn is_folder(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.folder"
    }

    pub fn is_shortcut(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.shortcut"
    }

    pub fn type_label(&self) -> &'static str {
        if self.is_folder() {
            "Folder"
        }  else {
            "File"
        }
    }

    /// Best-effort name of who last touched the file. Falls back to the
    /// email address if displayName isn't set, then "—". Currently unused
    /// in the UI but kept around — likely useful for future filters/sorts.
    #[allow(dead_code)]
    pub fn modified_by_label(&self) -> String {
        if let Some(user) = &self.last_modifying_user {
            if let Some(name) = &user.display_name {
                return name.clone();
            }
            if let Some(email) = &user.email_address {
                return email.clone();
            }
        }
        "—".to_string()
    }

    /// Render `modifiedTime` as "Mon DD, YYYY" (e.g. "Apr 25, 2026"). Drive
    /// returns a UTC ISO-8601 timestamp like "2026-04-25T10:00:00.000Z".
    /// We slice the date part directly rather than pull in a date crate.
    pub fn modified_time_label(&self) -> String {
        let mtime = match &self.modified_time {
            Some(s) if s.len() >= 10 => s,
            _ => return "—".to_string(),
        };

        let year: u16 = mtime[0..4].parse().unwrap_or(0);
        let month: u8 = mtime[5..7].parse().unwrap_or(0);
        let day: u8 = mtime[8..10].parse().unwrap_or(0);

        if year == 0 || month == 0 || day == 0 || month > 12 {
            return mtime.clone();
        }

        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        format!("{} {}, {}", MONTHS[(month - 1) as usize], day, year)
    }

    /// Full ISO timestamp for the cell `title` tooltip — gives power users
    /// access to the second-precision UTC time on hover.
    pub fn modified_time_iso(&self) -> String {
        self.modified_time.clone().unwrap_or_default()
    }
}

/// Drive's modifiedTime is ISO-8601 with millisecond precision and a `Z`
/// suffix, e.g. "2026-04-25T10:00:00.000Z". Lexicographic ordering of these
/// strings happens to match chronological ordering, which is convenient.
fn mtime_key(f: &DriveFile) -> &str {
    f.modified_time.as_deref().unwrap_or("")
}

/// One file annotated with the account it came from. The route handlers
/// build these so the rendered file list can show "Location" (which drive)
/// alongside the file metadata.
///
/// We deliberately use `SourceAccount` (a display-only projection) rather
/// than the full `Account` — the file list needs the avatar/email/name and
/// nothing else, so we don't carry `sub`, timestamps, or status into the
/// rendering layer.
#[derive(Debug, Clone)]
pub struct FileWithSource {
    pub file: DriveFile,
    pub account: crate::models::SourceAccount,
}

#[derive(Deserialize)]
struct FileListResponse {
    #[serde(default)]
    files: Vec<DriveFile>,
}

/// List files at the root of the drive owned by `access_token`.
///
/// `'root' in parents` is Drive's special alias for the root folder of the
/// authenticated user's "My Drive". Trashed files are filtered out
/// server-side.
pub async fn list_root_files(access_token: &str) -> Result<Vec<DriveFile>> {
    fetch(
        access_token,
        "'root' in parents and trashed = false",
        "list_root_files",
    )
    .await
}

/// Name-substring search across the drive. Multi-word queries become AND-ed
/// `name contains 'word'` clauses, so "vacation photos" matches files whose
/// name contains both "vacation" and "photos" (in any order, anywhere in the
/// name). Trashed files are filtered out.
pub async fn search_files(access_token: &str, user_query: &str) -> Result<Vec<DriveFile>> {
    let q = build_query(user_query);
    fetch(access_token, &q, "search_files").await
}

async fn fetch(access_token: &str, q: &str, op: &'static str) -> Result<Vec<DriveFile>> {
    let resp = reqwest::Client::new()
        .get(DRIVE_FILES_URL)
        .bearer_auth(access_token)
        .query(&[
            ("q", q),
            ("orderBy", "modifiedTime desc"),
            ("pageSize", &PAGE_SIZE.to_string()),
            ("fields", FIELDS),
            ("corpora", "user"),
        ])
        .send()
        .await
        .with_context(|| format!("Drive {op} request"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Drive {op} returned {status}: {body}");
    }
    let parsed: FileListResponse = resp
        .json()
        .await
        .with_context(|| format!("decoding Drive {op} response"))?;
    Ok(parsed.files)
}

/// Translate a free-text user query into Drive's q-parameter syntax.
///
/// Per Drive docs:
///   `'` → `\'`   (single quote escapes itself with a backslash)
///   `\` → `\\`   (and backslashes themselves must be escaped first)
fn build_query(user_query: &str) -> String {
    let words: Vec<String> = user_query
        .split_whitespace()
        .map(|w| w.replace('\\', "\\\\").replace('\'', "\\'"))
        .collect();

    if words.is_empty() {
        return "trashed = false".to_string();
    }

    let conditions: Vec<String> = words
        .iter()
        .map(|w| format!("name contains '{}'", w))
        .collect();

    format!("trashed = false and {}", conditions.join(" and "))
}

// ============================================================================
// k-way merge
// ============================================================================

/// Tuple stored in the heap: (modifiedTime, source-stream index, position
/// within that stream). We sort the heap by modifiedTime descending — since
/// `BinaryHeap` is a max-heap and ISO-8601 strings sort chronologically, we
/// just push raw strings and pop produces newest-first.
///
/// The stream index breaks ties deterministically when two files share an
/// exact mtime.
#[derive(Eq, PartialEq)]
struct HeapKey(String, usize, usize);

impl Ord for HeapKey {
    fn cmp(&self, other: &Self) -> Ordering {
        // Primary: mtime descending (BinaryHeap is max-heap, so this just
        // works — the bigger string wins, which for ISO-8601 means newer).
        match self.0.cmp(&other.0) {
            Ordering::Equal => other.1.cmp(&self.1), // tie-break by stream idx (stable)
            ord => ord,
        }
    }
}
impl PartialOrd for HeapKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// K-way merge of per-account streams, all individually pre-sorted by
/// modifiedTime desc, into one globally-sorted output.
///
/// Each input stream is one account's response from Drive. Drive returns
/// them sorted by `orderBy=modifiedTime desc` so we just need to interleave.
pub fn k_way_merge_by_mtime(streams: Vec<Vec<FileWithSource>>) -> Vec<FileWithSource> {
    let total: usize = streams.iter().map(|s| s.len()).sum();
    let mut out: Vec<FileWithSource> = Vec::with_capacity(total);

    let mut heap: BinaryHeap<HeapKey> = BinaryHeap::with_capacity(streams.len());
    for (i, stream) in streams.iter().enumerate() {
        if let Some(head) = stream.first() {
            heap.push(HeapKey(mtime_key(&head.file).to_string(), i, 0));
        }
    }

    while let Some(HeapKey(_, stream_idx, pos)) = heap.pop() {
        out.push(streams[stream_idx][pos].clone());
        let next_pos = pos + 1;
        if let Some(next) = streams[stream_idx].get(next_pos) {
            heap.push(HeapKey(
                mtime_key(&next.file).to_string(),
                stream_idx,
                next_pos,
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, mtime: &str) -> DriveFile {
        DriveFile {
            id: name.to_string(),
            name: name.to_string(),
            mime_type: "text/plain".into(),
            modified_time: Some(mtime.to_string()),
            web_view_link: None,
            last_modifying_user: None,
        }
    }
    fn fws(name: &str, mtime: &str, email: &str) -> FileWithSource {
        use crate::models::SourceAccount;
        FileWithSource {
            file: f(name, mtime),
            account: SourceAccount {
                email: email.to_string(),
                name: None,
                picture_url: None,
            },
        }
    }

    #[test]
    fn merges_two_sorted_streams() {
        let a = vec![
            fws("a3", "2026-04-25T03:00:00Z", "a@x"),
            fws("a2", "2026-04-25T02:00:00Z", "a@x"),
            fws("a1", "2026-04-25T01:00:00Z", "a@x"),
        ];
        let b = vec![
            fws("b2", "2026-04-25T02:30:00Z", "b@x"),
            fws("b1", "2026-04-25T01:30:00Z", "b@x"),
        ];
        let merged = k_way_merge_by_mtime(vec![a, b]);
        let names: Vec<&str> = merged.iter().map(|x| x.file.name.as_str()).collect();
        assert_eq!(names, vec!["a3", "b2", "a2", "b1", "a1"]);
    }

    #[test]
    fn handles_empty_streams() {
        assert!(k_way_merge_by_mtime(vec![]).is_empty());
        assert!(k_way_merge_by_mtime(vec![vec![]]).is_empty());
    }

    #[test]
    fn build_query_escapes_single_quotes() {
        assert_eq!(
            build_query("O'Brien"),
            "trashed = false and name contains 'O\\'Brien'"
        );
    }

    #[test]
    fn build_query_handles_multi_word() {
        assert_eq!(
            build_query("vacation photos"),
            "trashed = false and name contains 'vacation' and name contains 'photos'"
        );
    }

    #[test]
    fn build_query_empty_falls_back() {
        assert_eq!(build_query(""), "trashed = false");
        assert_eq!(build_query("   "), "trashed = false");
    }
}
