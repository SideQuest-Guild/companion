//! Detects whether a WoW client is currently running, so we never write SavedVariables (or stage an inbox) while
//! the game might also be writing it.

use std::ffi::OsStr;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// Whether any running process looks like a WoW client: `Wow.exe`, `WowClassic*.exe`, `WowB.exe`,
/// `World of Warcraft*` (case-insensitive; Wine/Proton show the same `.exe` name as native Windows).
pub fn is_wow_running() -> bool {
    let Ok(mut system) = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    ) else {
        tracing::warn!("could not initialize process list to check whether WoW is running");
        return false;
    };
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system.processes().values().any(|process| is_wow_process_name(process.name()))
}

fn is_wow_process_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    name == "wow"
        || name == "wowb"
        || name.starts_with("wowclassic")
        || name.starts_with("world of warcraft")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_known_wow_executable_names() {
        for name in [
            "Wow.exe",
            "WowClassic.exe",
            "WowClassicB.exe",
            "WowB.exe",
            "World of Warcraft.exe",
            "WOW.EXE",
        ] {
            assert!(
                is_wow_process_name(OsStr::new(name)),
                "expected {name} to be recognized as WoW"
            );
        }
    }

    #[test]
    fn rejects_unrelated_process_names() {
        for name in ["Discord.exe", "wowser.exe", "notwow.exe", "steam.exe"] {
            assert!(
                !is_wow_process_name(OsStr::new(name)),
                "expected {name} to NOT be recognized as WoW"
            );
        }
    }
}
