// Refresh-token encryption.
//
// Tokens are stored in SQLite as a BLOB containing:
//   nonce(12 bytes) || ciphertext_with_aead_tag(16+N bytes)
//
// Crypto: AES-256-GCM, the same primitive Google's own server-side products
// use for at-rest encryption. 12-byte random nonce per row, generated from
// the OS CSPRNG via `rand::thread_rng()`.
//
// Master key sourcing (12-factor friendly):
//   1. If the `OMNIDRIVE_MASTER_KEY` env var is set, decode it as base64
//      and use those bytes. Handy for containerized / production deploys
//      where secrets come from the orchestrator.
//   2. Otherwise, look for the `master.key` file in the data dir. Generate
//      one with mode 0600 if it doesn't exist.
//
// Threat model: this defends against accidental exposure of the SQLite file
// — stray cat, accidental git-add, screenshot, backup tarball. It does NOT
// defend against an attacker who has read access to the user's whole home
// directory; in that case they have both files (or the env) and the
// encryption is just theater. For the personal-local-app use case we're
// optimizing for, that's the right tradeoff: SQLCipher would be heavier,
// plaintext loses the "leaked .db is useless" property which is
// meaningfully nice.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand::rngs::OsRng;
use rand::RngCore;

/// Env var that, if set, overrides the file-based master key. Should be
/// 32 random bytes, base64-encoded:
///   `head -c 32 /dev/urandom | base64`
const ENV_VAR: &str = "OMNIDRIVE_MASTER_KEY";

const KEY_LEN: usize = 32;   // AES-256
const NONCE_LEN: usize = 12; // GCM standard

/// 32-byte master key, used to encrypt every refresh token at rest.
#[derive(Clone)]
pub struct MasterKey {
    key: [u8; KEY_LEN],
}

impl MasterKey {
    /// Resolve the master key, in order:
    ///
    ///   1. From `OMNIDRIVE_MASTER_KEY` env var (base64-decoded). Useful for
    ///      production / containerized deployments where the secret arrives
    ///      via the runtime environment.
    ///   2. From `path` if the file exists.
    ///   3. Otherwise generate fresh, write to `path` with mode 0600, and
    ///      log a one-time notice.
    pub fn load_or_generate(path: &Path) -> Result<Self> {
        if let Some(key) = Self::from_env()? {
            tracing::debug!("loaded master key from {ENV_VAR} env var");
            return Ok(key);
        }
        if path.exists() {
            return Self::load(path);
        }
        Self::generate_at(path)
    }

    /// Try to read the env var. Returns Ok(None) if unset, Err if set but
    /// malformed (so the user finds out loudly rather than silently falling
    /// back to a different key).
    fn from_env() -> Result<Option<Self>> {
        let raw = match std::env::var(ENV_VAR) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let bytes = B64
            .decode(trimmed)
            .with_context(|| format!("decoding {ENV_VAR} as base64"))?;
        if bytes.len() != KEY_LEN {
            anyhow::bail!(
                "{ENV_VAR} decoded to {} bytes; expected {} (32 random bytes, base64-encoded)",
                bytes.len(),
                KEY_LEN
            );
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Some(Self { key }))
    }

    fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading master key at {}", path.display()))?;
        if bytes.len() != KEY_LEN {
            anyhow::bail!(
                "master key file {} has unexpected length {} (expected {})",
                path.display(),
                bytes.len(),
                KEY_LEN
            );
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&bytes);
        tracing::debug!("loaded master key from {}", path.display());
        Ok(Self { key })
    }

    fn generate_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating master key parent dir {}", parent.display())
            })?;
        }

        let mut key = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut key);

        // Write atomically-ish: write to .tmp then rename. Best-effort —
        // a partial write here on first run is recoverable by deleting the
        // file and rerunning.
        let tmp_path = with_extension(path, "tmp");
        std::fs::write(&tmp_path, &key)
            .with_context(|| format!("writing master key tmp at {}", tmp_path.display()))?;

        // Restrict to 0600 on Unix BEFORE the rename so there's never a
        // window where a wider mode is visible at the final path.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&tmp_path, perms).with_context(|| {
                format!("chmod 0600 on {}", tmp_path.display())
            })?;
        }

        std::fs::rename(&tmp_path, path)
            .with_context(|| format!("renaming master key to {}", path.display()))?;

        tracing::info!(
            "generated new master key at {} — keep this file safe; \
             losing it makes existing connected accounts unrecoverable",
            path.display()
        );
        Ok(Self { key })
    }

    /// Encrypt `plaintext`, returning `nonce(12) || ciphertext_with_tag`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("aes-gcm encrypt failed: {e}"))?;

        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    /// Decrypt a `nonce(12) || ciphertext_with_tag` blob and return the
    /// plaintext.
    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < NONCE_LEN + 16 {
            // Need at least: nonce + GCM tag. (16 bytes is the tag length.)
            anyhow::bail!("encrypted blob too short ({} bytes)", blob.len());
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("aes-gcm decrypt failed: {e}"))
    }
}

/// Resolve the canonical path of the master key file, alongside the SQLite
/// db inside the platform's data dir.
pub fn master_key_path() -> Result<PathBuf> {
    let db = crate::db::db_path()?;
    let parent = db
        .parent()
        .context("db path has no parent directory")?
        .to_path_buf();
    Ok(parent.join("master.key"))
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension(ext);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_key() -> MasterKey {
        let mut key = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut key);
        MasterKey { key }
    }

    #[test]
    fn round_trip() {
        let mk = fresh_key();
        let plaintext = b"1//0gabc-secret-refresh-token-from-google";
        let blob = mk.encrypt(plaintext).expect("encrypt");
        let decoded = mk.decrypt(&blob).expect("decrypt");
        assert_eq!(decoded, plaintext);
    }

    #[test]
    fn ciphertext_is_not_plaintext() {
        let mk = fresh_key();
        let pt = b"some refresh token";
        let blob = mk.encrypt(pt).unwrap();
        assert!(!blob.windows(pt.len()).any(|w| w == pt));
    }

    #[test]
    fn nonce_makes_each_blob_unique() {
        // Two encrypts of the same plaintext must produce different blobs
        // (different nonces).
        let mk = fresh_key();
        let pt = b"identical plaintext";
        let a = mk.encrypt(pt).unwrap();
        let b = mk.encrypt(pt).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn tampered_blob_fails_to_decrypt() {
        let mk = fresh_key();
        let mut blob = mk.encrypt(b"hello").unwrap();
        // flip a bit in the ciphertext (after the nonce)
        blob[NONCE_LEN] ^= 0x01;
        assert!(mk.decrypt(&blob).is_err());
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let a = fresh_key();
        let b = fresh_key();
        let blob = a.encrypt(b"hello").unwrap();
        assert!(b.decrypt(&blob).is_err());
    }
}
