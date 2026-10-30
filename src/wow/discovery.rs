//! Finds WoW install roots, their flavor folders (`_classic_`, `_retail_`, ...), the SideQuest addon inside each,
//! and the SavedVariables accounts underneath. Every function here is pure over a root path so tests can run
//! against a temp directory tree instead of a real WoW install.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::state::{Account, Flavor, InstallSource, InstalledAddon, WowInstall};

/// Default WoW install roots to probe for this OS, before checking the registry (Windows) or `wow_roots`.
pub fn default_roots() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications/World of Warcraft")]
    } else if cfg!(target_os = "windows") {
        vec![
            PathBuf::from(r"C:\Program Files (x86)\World of Warcraft"),
            PathBuf::from(r"C:\Program Files\World of Warcraft"),
        ]
    } else if cfg!(target_os = "linux") {
        linux_default_roots()
    } else {
        Vec::new()
    }
}

fn linux_default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs_home() else { return Vec::new() };
    let mut roots = vec![
        home.join("Games/battlenet/drive_c/Program Files (x86)/World of Warcraft"),
        home.join(".wine/drive_c/Program Files (x86)/World of Warcraft"),
    ];
    roots.extend(glob_dirs(
        &home.join(".local/share/lutris"),
        "**/drive_c/Program Files (x86)/World of Warcraft",
    ));
    roots.extend(glob_dirs(
        &home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        "*/drive_c/Program Files (x86)/World of Warcraft",
    ));
    roots.extend(glob_dirs(
        &home.join(".steam/steam/steamapps/compatdata"),
        "*/pfx/drive_c/Program Files (x86)/World of Warcraft",
    ));
    roots
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// A tiny glob: `pattern` may contain `*` (one path segment) or `**` (any number of segments) as whole segments;
/// every other segment must match literally. Returns existing directories under `base` that match.
fn glob_dirs(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let segments: Vec<&str> = pattern.split('/').collect();
    let mut results = Vec::new();
    glob_walk(base, &segments, &mut results);
    results
}

fn glob_walk(current: &Path, segments: &[&str], results: &mut Vec<PathBuf>) {
    let Some((head, rest)) = segments.split_first() else {
        if current.is_dir() {
            results.push(current.to_path_buf());
        }
        return;
    };
    let Ok(entries) = std::fs::read_dir(current) else { return };
    match *head {
        "**" => {
            // Try matching zero segments of "**" (continue with rest here), and matching one level deeper while
            // staying in "**" mode, for every subdirectory.
            glob_walk(current, rest, results);
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    glob_walk(&path, segments, results);
                }
            }
        }
        "*" => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    glob_walk(&path, rest, results);
                }
            }
        }
        literal => {
            let candidate = current.join(literal);
            if candidate.is_dir() {
                glob_walk(&candidate, rest, results);
            }
        }
    }
}

/// Every `_*_` flavor folder directly under `root` (e.g. `_retail_`, `_classic_`, `_classic_beta_`). Forever's live
/// flavor folder name isn't fixed, so we glob rather than hardcode one.
pub fn find_flavor_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            e.path().is_dir() && name.starts_with('_') && name.ends_with('_') && name.len() > 2
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Read the SideQuest addon's installed version from `<flavor>/Interface/AddOns/SideQuest/SideQuest.toc`'s
/// `## Version:` line, if the addon is installed there.
pub fn find_addon(flavor_dir: &Path) -> InstalledAddon {
    let addon_dir = flavor_dir.join("Interface/AddOns/SideQuest");
    let toc_path = addon_dir.join("SideQuest.toc");
    match std::fs::read_to_string(&toc_path) {
        Ok(text) => match read_toc_version(&text) {
            Some(version) => InstalledAddon::Installed { version, path: addon_dir },
            None => InstalledAddon::Missing,
        },
        Err(_) => InstalledAddon::Missing,
    }
}

fn read_toc_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("## Version:") {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

/// Every `WTF/Account/<NAME>` folder under `flavor_dir` that has a `SavedVariables/SideQuest.lua` file.
/// `disabled_accounts` holds SavedVariables paths the user switched off in Settings.
pub fn find_accounts(flavor_dir: &Path, disabled_accounts: &[PathBuf]) -> Vec<Account> {
    let accounts_dir = flavor_dir.join("WTF/Account");
    let Ok(entries) = std::fs::read_dir(&accounts_dir) else { return Vec::new() };
    let mut accounts: Vec<Account> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let saved_variables = e.path().join("SavedVariables/SideQuest.lua");
            if !saved_variables.is_file() {
                return None;
            }
            let modified = std::fs::metadata(&saved_variables)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from);
            let enabled = !disabled_accounts.iter().any(|p| p == &saved_variables);
            Some(Account {
                name: e.file_name().to_string_lossy().into_owned(),
                saved_variables,
                enabled,
                modified,
                pending: None,
                export_hash: None,
                export_uploaded: false,
                inbox_staged: false,
                standings_generated_at: None,
            })
        })
        .collect();
    accounts.sort_by(|a, b| a.name.cmp(&b.name));
    accounts
}

/// Build a [`Flavor`] (addon + accounts) for every `_*_` folder under `root`. Does not read SavedVariables
/// contents; the engine fills in `Account::pending`/`export_hash`/etc. after parsing each file.
pub fn scan_install(
    root: &Path,
    source: InstallSource,
    disabled_accounts: &[PathBuf],
) -> Option<WowInstall> {
    if !root.is_dir() {
        return None;
    }
    let flavors: Vec<Flavor> = find_flavor_dirs(root)
        .into_iter()
        .map(|path| {
            let dir_name =
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let addon = find_addon(&path);
            let accounts = find_accounts(&path, disabled_accounts);
            Flavor { dir_name, path, addon, accounts }
        })
        .collect();
    Some(WowInstall { root: root.to_path_buf(), source, flavors })
}

/// Scan every default root plus `extra_roots` (deduplicated, defaults first), skipping ones that don't exist.
pub fn scan_all(extra_roots: &[PathBuf], disabled_accounts: &[PathBuf]) -> Vec<WowInstall> {
    let mut seen = std::collections::HashSet::new();
    let mut installs = Vec::new();
    for root in default_roots() {
        if seen.insert(root.clone())
            && let Some(install) = scan_install(&root, InstallSource::Detected, disabled_accounts)
        {
            installs.push(install);
        }
    }
    for root in extra_roots {
        if seen.insert(root.clone())
            && let Some(install) = scan_install(root, InstallSource::UserAdded, disabled_accounts)
        {
            installs.push(install);
        }
    }
    installs
}

#[cfg(target_os = "windows")]
pub fn registry_install_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key =
        hklm.open_subkey(r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft").ok()?;
    let install_path: String = key.get_value("InstallPath").ok()?;
    Some(normalize_registry_install_path(PathBuf::from(install_path)))
}

/// The registry's `InstallPath` may point at a flavor subfolder (e.g. `...\World of Warcraft\_retail_`) rather
/// than the root that contains every `_*_` folder; walk up to the root in that case.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn normalize_registry_install_path(path: PathBuf) -> PathBuf {
    let is_flavor_dir = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .is_some_and(|n| n.starts_with('_') && n.ends_with('_'));
    if is_flavor_dir { path.parent().map(Path::to_path_buf).unwrap_or(path) } else { path }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn make_flavor(
        root: &Path,
        flavor: &str,
        accounts: &[&str],
        addon_version: Option<&str>,
    ) -> PathBuf {
        let flavor_dir = root.join(flavor);
        for account in accounts {
            let sv_dir = flavor_dir.join("WTF/Account").join(account).join("SavedVariables");
            fs::create_dir_all(&sv_dir).unwrap();
            fs::write(sv_dir.join("SideQuest.lua"), "SideQuestDB = {}\n").unwrap();
        }
        if let Some(version) = addon_version {
            let addon_dir = flavor_dir.join("Interface/AddOns/SideQuest");
            fs::create_dir_all(&addon_dir).unwrap();
            fs::write(
                addon_dir.join("SideQuest.toc"),
                format!("## Interface: 110000\n## Version: {version}\n## Title: SideQuest\n"),
            )
            .unwrap();
        }
        flavor_dir
    }

    #[test]
    fn scans_two_flavors_with_accounts_and_one_missing_addon() {
        let dir = tempdir().unwrap();
        make_flavor(dir.path(), "_retail_", &["ACCOUNT1", "ACCOUNT2"], Some("v1.2.3"));
        make_flavor(dir.path(), "_classic_beta_", &["ACCOUNT1"], None);
        // Not a flavor folder: no leading/trailing underscores.
        fs::create_dir_all(dir.path().join("Updates")).unwrap();

        let install = scan_install(dir.path(), InstallSource::Detected, &[]).unwrap();
        assert_eq!(install.flavors.len(), 2);

        let retail = install.flavors.iter().find(|f| f.dir_name == "_retail_").unwrap();
        assert_eq!(retail.accounts.len(), 2);
        assert!(
            matches!(&retail.addon, InstalledAddon::Installed { version, .. } if version == "v1.2.3")
        );

        let classic = install.flavors.iter().find(|f| f.dir_name == "_classic_beta_").unwrap();
        assert_eq!(classic.accounts.len(), 1);
        assert!(matches!(classic.addon, InstalledAddon::Missing));
    }

    #[test]
    fn missing_root_returns_none() {
        let dir = tempdir().unwrap();
        assert!(
            scan_install(&dir.path().join("does-not-exist"), InstallSource::Detected, &[])
                .is_none()
        );
    }

    #[test]
    fn disabled_account_is_marked_disabled_but_still_listed() {
        let dir = tempdir().unwrap();
        make_flavor(dir.path(), "_retail_", &["ACCOUNT1"], None);
        let sv_path = dir.path().join("_retail_/WTF/Account/ACCOUNT1/SavedVariables/SideQuest.lua");

        let install =
            scan_install(dir.path(), InstallSource::Detected, std::slice::from_ref(&sv_path))
                .unwrap();
        let account = &install.flavors[0].accounts[0];
        assert!(!account.enabled);
        assert_eq!(account.saved_variables, sv_path);
    }

    #[test]
    fn dev_checkout_version_placeholder_is_read_literally() {
        let dir = tempdir().unwrap();
        make_flavor(dir.path(), "_retail_", &[], Some("@project-version@"));
        let install = scan_install(dir.path(), InstallSource::Detected, &[]).unwrap();
        assert!(
            matches!(&install.flavors[0].addon, InstalledAddon::Installed { version, .. } if version == "@project-version@")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn normalizes_a_flavor_subfolder_install_path_to_its_root() {
        let root = PathBuf::from(r"C:\Games\World of Warcraft");
        let flavor = root.join("_retail_");
        assert_eq!(normalize_registry_install_path(flavor), root);
        assert_eq!(normalize_registry_install_path(root.clone()), root);
    }
}
