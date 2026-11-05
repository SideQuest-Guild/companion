//! Loads and saves [`Settings`] (`settings.json` in the config dir) and [`SyncState`] (`state.json` in the data
//! dir, tracking which export hashes have already been uploaded so a restart does not re-upload them).

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::state::Settings;

const MIN_UI_SCALE: f32 = 0.75;
const MAX_UI_SCALE: f32 = 1.5;

/// Clamp whatever the UI or a hand-edited settings file sent us into a sane range.
pub fn clamp_settings(settings: &mut Settings) {
    settings.ui_scale = settings.ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
}

fn settings_path() -> Result<PathBuf> {
    Ok(paths::config_dir()?.join("settings.json"))
}

/// Load `settings.json`, tolerating unknown or missing fields (`Settings` derives `#[serde(default)]`). Falls back
/// to [`Settings::default`] if the file is missing or unreadable/corrupt; never panics or fails the caller.
pub fn load_settings() -> Settings {
    let mut settings = (|| -> Result<Settings> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Settings::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    })()
    .unwrap_or_else(|err| {
        tracing::warn!("could not load settings, using defaults: {err:#}");
        Settings::default()
    });
    clamp_settings(&mut settings);
    settings
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let mut settings = settings.clone();
    clamp_settings(&mut settings);
    let path = settings_path()?;
    let json = serde_json::to_string_pretty(&settings).context("serializing settings")?;
    atomic_write(&path, json.as_bytes())
}

/// Export upload bookkeeping, keyed by SHA-256 hex of the `SQX1:...` payload, plus the last app version an
/// automatic update tried to install.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncState {
    pub uploaded_hashes: HashMap<String, DateTime<Utc>>,
    /// Each version is installed automatically at most once: if the relaunched app still finds itself older
    /// (a failed swap, a mislabelled release), it leaves the update to the Install button instead of looping.
    pub auto_update_attempted: Option<String>,
}

impl SyncState {
    pub fn is_uploaded(&self, hash: &str) -> bool {
        self.uploaded_hashes.contains_key(hash)
    }

    pub fn mark_uploaded(&mut self, hash: &str, at: DateTime<Utc>) {
        self.uploaded_hashes.insert(hash.to_owned(), at);
    }
}

fn state_path() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("state.json"))
}

pub fn load_state() -> SyncState {
    (|| -> Result<SyncState> {
        let path = state_path()?;
        if !path.exists() {
            return Ok(SyncState::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    })()
    .unwrap_or_else(|err| {
        tracing::warn!("could not load sync state, starting fresh: {err:#}");
        SyncState::default()
    })
}

pub fn save_state(state: &SyncState) -> Result<()> {
    let path = state_path()?;
    let json = serde_json::to_string_pretty(state).context("serializing sync state")?;
    atomic_write(&path, json.as_bytes())
}

/// Write `bytes` to `path` via a temp file in the same directory plus a rename, so a crash mid-write never leaves a
/// truncated or partially-written file behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("destination path has no parent directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("creating temp file in {}", dir.display()))?;
    tmp.write_all(bytes).context("writing temp file")?;
    tmp.flush().context("flushing temp file")?;
    tmp.persist(path)
        .map_err(|err| err.error)
        .with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}
