//! Append-only sync/update history, newest first, capped at [`MAX_ENTRIES`]. Persisted as `history.json` in the
//! data dir. `state::HistoryEntry` has no `serde` derive (it's UI-facing, not a storage format), so this module
//! keeps its own serializable mirror and converts both ways.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::atomic_write;
use crate::paths;
use crate::state::{HistoryEntry, HistoryKind, Outcome};

pub const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntryDto {
    at: DateTime<Utc>,
    kind: HistoryKind,
    outcome: Outcome,
    subject: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl From<&HistoryEntry> for HistoryEntryDto {
    fn from(e: &HistoryEntry) -> Self {
        Self {
            at: e.at,
            kind: e.kind,
            outcome: e.outcome,
            subject: e.subject.clone(),
            summary: e.summary.clone(),
            detail: e.detail.clone(),
        }
    }
}

impl From<HistoryEntryDto> for HistoryEntry {
    fn from(d: HistoryEntryDto) -> Self {
        Self {
            at: d.at,
            kind: d.kind,
            outcome: d.outcome,
            subject: d.subject,
            summary: d.summary,
            detail: d.detail,
        }
    }
}

fn history_path() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("history.json"))
}

/// Load the history, newest first. Missing or corrupt files are treated as empty history rather than an error.
pub fn load() -> Vec<HistoryEntry> {
    (|| -> Result<Vec<HistoryEntry>> {
        let path = history_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let dtos: Vec<HistoryEntryDto> =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(dtos.into_iter().map(HistoryEntry::from).collect())
    })()
    .unwrap_or_else(|err| {
        tracing::warn!("could not load history, starting fresh: {err:#}");
        Vec::new()
    })
}

/// Overwrite `history.json` with `entries` (expected newest first), capped at [`MAX_ENTRIES`].
pub fn save(entries: &[HistoryEntry]) -> Result<()> {
    let capped = &entries[..entries.len().min(MAX_ENTRIES)];
    let dtos: Vec<HistoryEntryDto> = capped.iter().map(HistoryEntryDto::from).collect();
    let json = serde_json::to_string_pretty(&dtos).context("serializing history")?;
    atomic_write(&history_path()?, json.as_bytes())
}

/// Insert `entry` at the front of `entries` (newest first) and cap at [`MAX_ENTRIES`]. Does not persist; call
/// [`save`] afterwards.
pub fn append(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(subject: &str) -> HistoryEntry {
        HistoryEntry {
            at: Utc::now(),
            kind: HistoryKind::Upload,
            outcome: Outcome::Success,
            subject: subject.to_owned(),
            summary: "2 sessions".to_owned(),
            detail: None,
        }
    }

    #[test]
    fn append_caps_at_max_entries() {
        let mut entries = Vec::new();
        for i in 0..(MAX_ENTRIES + 10) {
            append(&mut entries, entry(&i.to_string()));
        }
        assert_eq!(entries.len(), MAX_ENTRIES);
        // Most recently appended is first.
        assert_eq!(entries[0].subject, (MAX_ENTRIES + 9).to_string());
    }

    #[test]
    fn roundtrips_through_dto() {
        let e = entry("test");
        let dto = HistoryEntryDto::from(&e);
        let back: HistoryEntry = dto.into();
        assert_eq!(back, e);
    }
}
