//! Keeps a flavor's `SideQuest_Inbox` addon folder current with the latest game import. Unlike the old
//! SavedVariables inbox, WoW reads addon `.lua` files fresh from disk at login and every `/reload` and never
//! writes them back, so [`write`] can run whether or not WoW is running: a `/reload` in game always picks up
//! the newest site data.

use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::config::atomic_write;

const ADDON_DIR_NAME: &str = "SideQuest_Inbox";
const TOC_FILE_NAME: &str = "SideQuest_Inbox.toc";
const LUA_FILE_NAME: &str = "Inbox.lua";
/// Used when the `SideQuest` addon's `.toc` is missing or has no readable `## Interface:` line.
const FALLBACK_INTERFACE: &str = "16001";

/// What's currently written in a flavor's `Inbox.lua`, read back from the file this module writes (not
/// general Lua -- just the two fields it puts there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxStatus {
    pub generated_at: DateTime<Utc>,
    pub staged_at: DateTime<Utc>,
}

/// The addon folder this module owns, under `flavor_dir/Interface/AddOns`.
fn addon_dir(flavor_dir: &Path) -> std::path::PathBuf {
    flavor_dir.join("Interface").join("AddOns").join(ADDON_DIR_NAME)
}

/// Write (or refresh) `flavor_dir`'s `SideQuest_Inbox` addon folder per the inbox contract: creates the folder
/// if missing, rewrites the `.toc` only when its content would differ (e.g. the `SideQuest` addon's Interface
/// line changed), and always replaces `Inbox.lua` atomically. Refuses (without writing anything) an import
/// whose `generated_at` is older than the one already staged; equal or newer is fine.
pub fn write(
    flavor_dir: &Path,
    payload: &str,
    generated_at: DateTime<Utc>,
    staged_at: DateTime<Utc>,
) -> Result<()> {
    if let Some(status) = read_status(flavor_dir)
        && generated_at < status.generated_at
    {
        bail!(
            "refusing to stage an import generated at {generated_at} because it's older than the inbox's \
             current one ({})",
            status.generated_at
        );
    }

    let dir = addon_dir(flavor_dir);
    let toc_path = dir.join(TOC_FILE_NAME);
    let toc = render_toc(&interface_line(flavor_dir));
    let toc_differs =
        std::fs::read(&toc_path).ok().is_none_or(|existing| existing != toc.as_bytes());
    if toc_differs {
        atomic_write(&toc_path, toc.as_bytes()).context("writing SideQuest_Inbox.toc")?;
    }

    let lua = render_lua(payload, generated_at, staged_at);
    atomic_write(&dir.join(LUA_FILE_NAME), lua.as_bytes()).context("writing Inbox.lua")
}

/// The flavor's `SideQuest` addon's `## Interface:` value, or [`FALLBACK_INTERFACE`] if its `.toc` is missing,
/// unreadable, or has no such line.
fn interface_line(flavor_dir: &Path) -> String {
    let toc_path =
        flavor_dir.join("Interface").join("AddOns").join("SideQuest").join("SideQuest.toc");
    std::fs::read_to_string(&toc_path)
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.trim().strip_prefix("## Interface:").map(|rest| rest.trim().to_owned())
            })
        })
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| FALLBACK_INTERFACE.to_owned())
}

fn render_toc(interface: &str) -> String {
    format!(
        "## Interface: {interface}\n\
         ## Dependencies: SideQuest\n\
         ## Title: SideQuest Inbox\n\
         ## Notes: Site data SideQuest Companion keeps here for the SideQuest addon. Managed by the companion.\n\
         ## Author: SideQuest Companion\n\
         ## Version: 1\n\
         {LUA_FILE_NAME}\n"
    )
}

fn render_lua(payload: &str, generated_at: DateTime<Utc>, staged_at: DateTime<Utc>) -> String {
    format!(
        "-- Written by SideQuest Companion; replaced on every sync. Don't edit.\n\
         SideQuestCompanionInbox = {{\n\
         \x20   payload = \"{payload}\",\n\
         \x20   generated_at = {generated_at},\n\
         \x20   staged_at = {staged_at},\n\
         }}\n",
        payload = lua_escape(payload),
        generated_at = generated_at.timestamp(),
        staged_at = staged_at.timestamp(),
    )
}

/// Escapes a string for a Lua double-quoted literal: backslashes, quotes, newlines, carriage returns, and any
/// other ASCII control character (the payload is base64 today, but this holds even if that ever changes).
fn lua_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// What's currently staged in `flavor_dir`'s `Inbox.lua`, if it exists and both fields parse.
pub fn read_status(flavor_dir: &Path) -> Option<InboxStatus> {
    let text = std::fs::read_to_string(addon_dir(flavor_dir).join(LUA_FILE_NAME)).ok()?;
    let generated_at = DateTime::<Utc>::from_timestamp(parse_field(&text, "generated_at")?, 0)?;
    let staged_at = DateTime::<Utc>::from_timestamp(parse_field(&text, "staged_at")?, 0)?;
    Some(InboxStatus { generated_at, staged_at })
}

/// Reads a `name = 123,` (or `name = 123`) line out of the file this module writes, no general Lua parsing.
fn parse_field(text: &str, name: &str) -> Option<i64> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(name)?.trim_start();
        let rest = rest.strip_prefix('=')?.trim();
        rest.trim_end_matches(',').parse::<i64>().ok()
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;

    fn write_sidequest_toc(flavor_dir: &Path, interface: &str) {
        let addon_dir = flavor_dir.join("Interface").join("AddOns").join("SideQuest");
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::write(
            addon_dir.join("SideQuest.toc"),
            format!("## Interface: {interface}\n## Version: 1.0.0\n## Title: SideQuest\n"),
        )
        .unwrap();
    }

    #[test]
    fn lua_escape_handles_backslashes_quotes_newlines_and_control_chars() {
        assert_eq!(lua_escape(r#"a\b"c"#), r#"a\\b\"c"#);
        assert_eq!(lua_escape("line1\nline2\r\n"), "line1\\nline2\\r\\n");
        assert_eq!(lua_escape("\x01\x1f"), "\\001\\031");
        assert_eq!(lua_escape("SQI1:AbC+/=="), "SQI1:AbC+/==");
    }

    #[test]
    fn write_creates_the_addon_folder_and_both_files() {
        let dir = tempdir().unwrap();
        write_sidequest_toc(dir.path(), "110105");
        let generated_at = Utc.timestamp_opt(1_794_555_642, 0).unwrap();
        let staged_at = Utc.timestamp_opt(1_794_555_700, 0).unwrap();

        write(dir.path(), "SQI1:abc123", generated_at, staged_at).unwrap();

        let toc = std::fs::read_to_string(
            dir.path().join("Interface/AddOns/SideQuest_Inbox/SideQuest_Inbox.toc"),
        )
        .unwrap();
        assert_eq!(
            toc,
            "## Interface: 110105\n\
             ## Dependencies: SideQuest\n\
             ## Title: SideQuest Inbox\n\
             ## Notes: Site data SideQuest Companion keeps here for the SideQuest addon. Managed by the companion.\n\
             ## Author: SideQuest Companion\n\
             ## Version: 1\n\
             Inbox.lua\n"
        );
        let lua =
            std::fs::read_to_string(dir.path().join("Interface/AddOns/SideQuest_Inbox/Inbox.lua"))
                .unwrap();
        assert_eq!(
            lua,
            "-- Written by SideQuest Companion; replaced on every sync. Don't edit.\n\
             SideQuestCompanionInbox = {\n\
             \x20   payload = \"SQI1:abc123\",\n\
             \x20   generated_at = 1794555642,\n\
             \x20   staged_at = 1794555700,\n\
             }\n"
        );
        assert_eq!(read_status(dir.path()), Some(InboxStatus { generated_at, staged_at }));
    }

    #[test]
    fn write_falls_back_to_16001_without_a_readable_sidequest_toc() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "SQI1:abc",
            Utc.timestamp_opt(1, 0).unwrap(),
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap();
        let toc = std::fs::read_to_string(
            dir.path().join("Interface/AddOns/SideQuest_Inbox/SideQuest_Inbox.toc"),
        )
        .unwrap();
        assert!(toc.starts_with("## Interface: 16001\n"), "{toc}");
    }

    #[test]
    fn rewrites_the_toc_when_sidequests_interface_line_changes() {
        let dir = tempdir().unwrap();
        write_sidequest_toc(dir.path(), "110105");
        write(
            dir.path(),
            "SQI1:abc",
            Utc.timestamp_opt(1, 0).unwrap(),
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap();

        write_sidequest_toc(dir.path(), "110200");
        write(
            dir.path(),
            "SQI1:abc",
            Utc.timestamp_opt(3, 0).unwrap(),
            Utc.timestamp_opt(4, 0).unwrap(),
        )
        .unwrap();

        let toc = std::fs::read_to_string(
            dir.path().join("Interface/AddOns/SideQuest_Inbox/SideQuest_Inbox.toc"),
        )
        .unwrap();
        assert!(toc.starts_with("## Interface: 110200\n"), "{toc}");
    }

    #[test]
    fn rewrites_an_old_toc_that_predates_the_dependencies_line() {
        let dir = tempdir().unwrap();
        write_sidequest_toc(dir.path(), "110105");
        let addon_dir = dir.path().join("Interface/AddOns/SideQuest_Inbox");
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::write(
            addon_dir.join("SideQuest_Inbox.toc"),
            "## Interface: 110105\n\
             ## Title: SideQuest Inbox\n\
             ## Notes: Site data SideQuest Companion keeps here for the SideQuest addon. Managed by the companion.\n\
             ## Author: SideQuest Companion\n\
             ## Version: 1\n\
             Inbox.lua\n",
        )
        .unwrap();

        write(
            dir.path(),
            "SQI1:abc",
            Utc.timestamp_opt(1, 0).unwrap(),
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap();

        let toc = std::fs::read_to_string(addon_dir.join("SideQuest_Inbox.toc")).unwrap();
        assert!(
            toc.contains("## Dependencies: SideQuest\n"),
            "an old installed .toc without the line must be rewritten to add it: {toc}"
        );
    }

    #[test]
    fn refuses_an_import_older_than_the_one_already_staged() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "SQI1:newer",
            Utc.timestamp_opt(1000, 0).unwrap(),
            Utc.timestamp_opt(1001, 0).unwrap(),
        )
        .unwrap();

        let err = write(
            dir.path(),
            "SQI1:older",
            Utc.timestamp_opt(500, 0).unwrap(),
            Utc.timestamp_opt(1002, 0).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("older"), "{err}");

        let lua =
            std::fs::read_to_string(dir.path().join("Interface/AddOns/SideQuest_Inbox/Inbox.lua"))
                .unwrap();
        assert!(lua.contains("SQI1:newer"), "the refused write must leave the file untouched");
    }

    #[test]
    fn an_import_with_the_same_generated_at_is_accepted() {
        let dir = tempdir().unwrap();
        let generated_at = Utc.timestamp_opt(1000, 0).unwrap();
        write(dir.path(), "SQI1:first", generated_at, Utc.timestamp_opt(1001, 0).unwrap()).unwrap();
        write(dir.path(), "SQI1:second", generated_at, Utc.timestamp_opt(1002, 0).unwrap())
            .unwrap();

        let lua =
            std::fs::read_to_string(dir.path().join("Interface/AddOns/SideQuest_Inbox/Inbox.lua"))
                .unwrap();
        assert!(
            lua.contains("SQI1:second"),
            "equal generated_at is accepted and replaces the file"
        );
    }

    #[test]
    fn write_is_atomic_no_partial_file_left_behind_on_success() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "SQI1:abc",
            Utc.timestamp_opt(1, 0).unwrap(),
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap();
        let entries: Vec<_> =
            std::fs::read_dir(dir.path().join("Interface/AddOns/SideQuest_Inbox"))
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            entries.into_iter().collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([
                "SideQuest_Inbox.toc".to_owned(),
                "Inbox.lua".to_owned()
            ]),
            "no leftover temp files"
        );
    }
}
