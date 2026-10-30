//! The sync token and private calendar link live in the OS keyring (service
//! `com.sidequest-guild.companion`, entries `sync-token` and `calendar-url`). When the platform has no keyring
//! backend (e.g. Linux without a Secret Service), we fall back to a 0600 `secrets.json` file in the config dir.
//! Neither secret is ever written to `settings.json`, `state.json`, `history.json` or a log line.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};

use crate::config::atomic_write;
use crate::paths;

pub const SERVICE: &str = "com.sidequest-guild.companion";
const TOKEN_KEY: &str = "sync-token";
const CALENDAR_KEY: &str = "calendar-url";

static FILE_FALLBACK: AtomicBool = AtomicBool::new(false);

/// Whether we had to fall back to the file store because the platform keyring was unavailable. The engine checks
/// this once at startup (after the first read/write attempt) and surfaces a warning [`crate::state::Notice`].
pub fn using_file_fallback() -> bool {
    FILE_FALLBACK.load(Ordering::Relaxed)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FileSecrets {
    #[serde(rename = "sync-token", skip_serializing_if = "Option::is_none")]
    sync_token: Option<String>,
    #[serde(rename = "calendar-url", skip_serializing_if = "Option::is_none")]
    calendar_url: Option<String>,
}

fn secrets_file_path() -> Result<PathBuf> {
    Ok(paths::config_dir()?.join("secrets.json"))
}

fn load_file_secrets() -> Result<FileSecrets> {
    let path = secrets_file_path()?;
    if !path.exists() {
        return Ok(FileSecrets::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn save_file_secrets(secrets: &FileSecrets) -> Result<()> {
    let path = secrets_file_path()?;
    let json = serde_json::to_string_pretty(secrets).context("serializing secrets")?;
    atomic_write(&path, json.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }
    Ok(())
}

/// A keyring error that means "there is no usable platform credential store here", as opposed to "there is a store
/// but this entry isn't in it" ([`keyring::Error::NoEntry`], which is a legitimate empty result).
fn is_no_backend(err: &keyring::Error) -> bool {
    matches!(err, keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_))
}

fn note_fallback(err: &keyring::Error) {
    FILE_FALLBACK.store(true, Ordering::Relaxed);
    tracing::warn!("OS keyring unavailable ({err}); falling back to file-based secret storage");
}

fn keyring_get(key: &str) -> Result<Option<String>, keyring::Error> {
    let entry = Entry::new(SERVICE, key)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err),
    }
}

fn keyring_set(key: &str, value: &str) -> Result<(), keyring::Error> {
    Entry::new(SERVICE, key)?.set_password(value)
}

fn keyring_clear(key: &str) -> Result<(), keyring::Error> {
    match Entry::new(SERVICE, key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err),
    }
}

fn get(
    key: &str,
    file_value: impl FnOnce(&FileSecrets) -> Option<String>,
) -> Result<Option<String>> {
    if !using_file_fallback() {
        match keyring_get(key) {
            Ok(value) => return Ok(value),
            Err(err) if is_no_backend(&err) => note_fallback(&err),
            Err(err) => return Err(err.into()),
        }
    }
    Ok(file_value(&load_file_secrets()?))
}

fn set(key: &str, value: &str, file_set: impl FnOnce(&mut FileSecrets, String)) -> Result<()> {
    if !using_file_fallback() {
        match keyring_set(key, value) {
            Ok(()) => return Ok(()),
            Err(err) if is_no_backend(&err) => note_fallback(&err),
            Err(err) => return Err(err.into()),
        }
    }
    let mut secrets = load_file_secrets()?;
    file_set(&mut secrets, value.to_owned());
    save_file_secrets(&secrets)
}

fn clear(key: &str, file_clear: impl FnOnce(&mut FileSecrets)) -> Result<()> {
    if !using_file_fallback() {
        match keyring_clear(key) {
            Ok(()) => return Ok(()),
            Err(err) if is_no_backend(&err) => note_fallback(&err),
            Err(err) => return Err(err.into()),
        }
    }
    let mut secrets = load_file_secrets()?;
    file_clear(&mut secrets);
    save_file_secrets(&secrets)
}

pub fn get_token() -> Result<Option<String>> {
    get(TOKEN_KEY, |f| f.sync_token.clone())
}

pub fn set_token(token: &str) -> Result<()> {
    set(TOKEN_KEY, token, |f, v| f.sync_token = Some(v))
}

pub fn clear_token() -> Result<()> {
    clear(TOKEN_KEY, |f| f.sync_token = None)
}

pub fn get_calendar_url() -> Result<Option<String>> {
    get(CALENDAR_KEY, |f| f.calendar_url.clone())
}

pub fn set_calendar_url(url: &str) -> Result<()> {
    set(CALENDAR_KEY, url, |f, v| f.calendar_url = Some(v))
}

pub fn clear_calendar_url() -> Result<()> {
    clear(CALENDAR_KEY, |f| f.calendar_url = None)
}

/// Show the prefix and last 4 characters of a token, e.g. `sqs_…a1B2`. Never show the rest.
pub fn masked(token: &str) -> String {
    let prefix = token.get(..4).unwrap_or(token);
    let suffix = if token.len() > 4 { &token[token.len() - 4..] } else { "" };
    format!("{prefix}\u{2026}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_token_prefix_and_suffix() {
        assert_eq!(masked("sqs_abcdefghijklmnopqrstuvwxyza1B2"), "sqs_\u{2026}a1B2");
    }

    #[test]
    fn masks_short_token_without_panicking() {
        assert_eq!(masked("abc"), "abc\u{2026}");
    }
}
