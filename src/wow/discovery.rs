//! Finds WoW install roots, their flavor folders (`_classic_`, `_retail_`, ...), the SideQuest addon inside each,
//! and the SavedVariables accounts underneath. Every function here is pure over a root path so tests can run
//! against a temp directory tree instead of a real WoW install.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::state::{Account, Flavor, InstallSource, InstalledAddon, WowInstall};

/// Usual WoW install roots to probe for this OS. [`detected_roots`] adds the ones Battle.net and the registry know.
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

/// Every WoW install root this machine tells us about: the usual folders, every WoW game in Battle.net's
/// `product.db` (which covers installs on any drive or folder), and on Windows the registry's `InstallPath`.
pub fn detected_roots() -> Vec<PathBuf> {
    let mut roots = default_roots();
    roots.extend(battle_net_install_roots());
    #[cfg(target_os = "windows")]
    roots.extend(registry_install_path());
    roots
}

/// Where the Battle.net agent keeps its record of installed games.
fn battle_net_product_db() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let program_data = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        Some(program_data.join(r"Battle.net\Agent\product.db"))
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from("/Users/Shared/Battle.net/Agent/product.db"))
    } else {
        None
    }
}

fn battle_net_install_roots() -> Vec<PathBuf> {
    let Some(path) = battle_net_product_db() else { return Vec::new() };
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    parse_product_db(&bytes)
}

/// The install folders of every WoW game (product codes `wow`, `wowf`, `wow_classic`, ...) in a Battle.net
/// `product.db`. It's a protobuf: field 1 repeats one message per game, holding the product code in field 2 and a
/// settings message in field 3 whose field 1 is the install folder.
fn parse_product_db(bytes: &[u8]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (field, product) in length_delimited_fields(bytes) {
        if field != 1 {
            continue;
        }
        let product = length_delimited_fields(product);
        if !product.iter().any(|&(field, code)| field == 2 && code.starts_with(b"wow")) {
            continue;
        }
        for &(_, settings) in product.iter().filter(|&&(field, _)| field == 3) {
            for (field, path) in length_delimited_fields(settings) {
                if field == 1
                    && let Ok(path) = std::str::from_utf8(path)
                    && !path.is_empty()
                {
                    let root = install_root(PathBuf::from(path));
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            }
        }
    }
    roots
}

/// The length-delimited fields (strings and nested messages) of one protobuf message, as (field number, bytes).
/// Skips number fields and stops at the first malformed one.
fn length_delimited_fields(mut bytes: &[u8]) -> Vec<(u64, &[u8])> {
    let mut fields = Vec::new();
    while let Some(key) = read_varint(&mut bytes) {
        let skip = match key & 7 {
            0 => read_varint(&mut bytes).map(|_| 0),
            1 => Some(8),
            5 => Some(4),
            2 => match read_varint(&mut bytes).and_then(|len| usize::try_from(len).ok()) {
                Some(len) if len <= bytes.len() => {
                    fields.push((key >> 3, &bytes[..len]));
                    Some(len)
                }
                _ => None,
            },
            _ => None,
        };
        match skip {
            Some(skip) if skip <= bytes.len() => bytes = &bytes[skip..],
            _ => break,
        }
    }
    fields
}

fn read_varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

fn is_flavor_name(name: &str) -> bool {
    name.starts_with('_') && name.ends_with('_') && name.len() > 2
}

/// The install root holding the `_*_` flavor folders, for a path that may point inside one: Battle.net and the
/// registry sometimes record `...\World of Warcraft\_retail_`, and people pick `_forever_` or its AddOns folder
/// by hand.
pub fn install_root(path: PathBuf) -> PathBuf {
    let mut root = PathBuf::new();
    for component in path.components() {
        if is_flavor_name(&component.as_os_str().to_string_lossy()) {
            return if root.as_os_str().is_empty() { path } else { root };
        }
        root.push(component);
    }
    path
}

/// The SideQuest addon only runs on WoW: Forever, so only its flavor folders count: `_forever_` and any test client
/// beside it (`_forever_ptr_`, ...). `_retail_`, `_classic_` and the rest are left alone.
fn is_supported_flavor(name: &str) -> bool {
    is_flavor_name(name) && name.starts_with("_forever")
}

/// Every Forever flavor folder directly under `root` (see [`is_supported_flavor`]).
pub fn find_flavor_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir() && is_supported_flavor(&e.file_name().to_string_lossy()))
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

/// The `## Version:` value without the tag's leading `v` (`v0.5.2` reads as `0.5.2`), the same form as a release's
/// version, so the two compare and display alike.
fn read_toc_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("## Version:") {
            let version = rest.trim();
            return Some(version.strip_prefix('v').unwrap_or(version).to_owned());
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

/// Scan `user_roots` and, with `auto_detect`, every [`detected_roots`] entry first. Paths inside a flavor folder
/// count as their install root; each root is scanned once and missing ones are skipped.
pub fn scan_all(
    user_roots: &[PathBuf],
    auto_detect: bool,
    disabled_accounts: &[PathBuf],
) -> Vec<WowInstall> {
    let detected = if auto_detect { detected_roots() } else { Vec::new() };
    let roots = detected
        .into_iter()
        .map(|root| (root, InstallSource::Detected))
        .chain(user_roots.iter().map(|root| (root.clone(), InstallSource::UserAdded)));
    let mut seen = std::collections::HashSet::new();
    let mut installs = Vec::new();
    for (root, source) in roots {
        let root = install_root(root);
        if seen.insert(root.clone())
            && let Some(install) = scan_install(&root, source, disabled_accounts)
        {
            installs.push(install);
        }
    }
    installs
}

/// Registry locations that may hold WoW's `InstallPath`, most to least likely to have it: the native view a
/// modern 64-bit Battle.net Agent/installer writes to, then `WOW6432Node`, where Windows redirects a 32-bit
/// process's `SOFTWARE` writes and where older/legacy installers wrote directly. Checking only the second
/// (the previous behaviour) misses every native 64-bit install, which is the common case today.
#[cfg(target_os = "windows")]
const REGISTRY_INSTALL_KEYS: &[&str] = &[
    r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
    r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
];

#[cfg(target_os = "windows")]
fn registry_install_path_in(hive: winreg::RegKey) -> Option<PathBuf> {
    REGISTRY_INSTALL_KEYS.iter().find_map(|subkey| {
        let key = hive.open_subkey(subkey).ok()?;
        let install_path: String = key.get_value("InstallPath").ok()?;
        Some(PathBuf::from(install_path))
    })
}

#[cfg(target_os = "windows")]
fn registry_install_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    registry_install_path_in(RegKey::predef(HKEY_LOCAL_MACHINE))
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
    fn scans_only_forever_flavors_and_reads_the_addon_version_without_its_v() {
        let dir = tempdir().unwrap();
        make_flavor(dir.path(), "_forever_", &["ACCOUNT1", "ACCOUNT2"], Some("v1.2.3"));
        make_flavor(dir.path(), "_forever_ptr_", &["ACCOUNT1"], None);
        // Other games' flavor folders, even with an account and the addon, aren't SideQuest's to manage.
        make_flavor(dir.path(), "_retail_", &["ACCOUNT1"], Some("v1.0.0"));
        make_flavor(dir.path(), "_classic_beta_", &["ACCOUNT1"], None);
        // Not a flavor folder: no leading/trailing underscores.
        fs::create_dir_all(dir.path().join("Updates")).unwrap();

        let install = scan_install(dir.path(), InstallSource::Detected, &[]).unwrap();
        let names: Vec<&str> = install.flavors.iter().map(|f| f.dir_name.as_str()).collect();
        assert_eq!(names, ["_forever_", "_forever_ptr_"]);

        let live = &install.flavors[0];
        assert_eq!(live.accounts.len(), 2);
        assert!(
            matches!(&live.addon, InstalledAddon::Installed { version, .. } if version == "1.2.3")
        );

        let ptr = &install.flavors[1];
        assert_eq!(ptr.accounts.len(), 1);
        assert!(matches!(ptr.addon, InstalledAddon::Missing));
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
        make_flavor(dir.path(), "_forever_", &["ACCOUNT1"], None);
        let sv_path =
            dir.path().join("_forever_/WTF/Account/ACCOUNT1/SavedVariables/SideQuest.lua");

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
        make_flavor(dir.path(), "_forever_", &[], Some("@project-version@"));
        let install = scan_install(dir.path(), InstallSource::Detected, &[]).unwrap();
        assert!(
            matches!(&install.flavors[0].addon, InstalledAddon::Installed { version, .. } if version == "@project-version@")
        );
    }

    #[test]
    fn a_path_inside_a_flavor_folder_counts_as_its_install_root() {
        let root = PathBuf::from("/Games/World of Warcraft");
        assert_eq!(install_root(root.join("_retail_")), root);
        assert_eq!(install_root(root.join("_forever_/Interface/AddOns")), root);
        assert_eq!(install_root(root.clone()), root);
    }

    fn proto_field(field: u64, bytes: &[u8]) -> Vec<u8> {
        let mut out =
            vec![u8::try_from(field << 3 | 2).unwrap(), u8::try_from(bytes.len()).unwrap()];
        out.extend_from_slice(bytes);
        out
    }

    fn product(code: &str, install_path: &str) -> Vec<u8> {
        let mut settings = proto_field(1, install_path.as_bytes());
        settings.extend(proto_field(13, b"_forever_"));
        let mut message = proto_field(1, code.as_bytes());
        message.extend(proto_field(2, code.as_bytes()));
        message.extend([0x38, 0x00]); // field 7, a number: skipped
        message.extend(proto_field(3, &settings));
        proto_field(1, &message)
    }

    #[test]
    fn battle_net_product_db_lists_every_wow_install_folder() {
        let mut db = product("bna", "C:/Program Files (x86)/Battle.net");
        db.extend(product("wowf", "D:/Games/World of Warcraft"));
        db.extend(product("wow_classic", "D:/Games/World of Warcraft/_classic_"));
        db.extend(product("wow", "E:/World of Warcraft"));
        assert_eq!(
            parse_product_db(&db),
            vec![
                PathBuf::from("D:/Games/World of Warcraft"),
                PathBuf::from("E:/World of Warcraft")
            ]
        );
    }

    #[test]
    fn a_truncated_product_db_yields_what_it_could_read() {
        let mut db = product("wowf", "D:/Games/World of Warcraft");
        let mut second = product("wow", "E:/World of Warcraft");
        second.truncate(second.len() - 3);
        db.extend(second);
        assert_eq!(parse_product_db(&db), vec![PathBuf::from("D:/Games/World of Warcraft")]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_install_path_prefers_the_native_key_over_wow6432node() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;

        // HKCU needs no admin rights, unlike the HKLM the production code reads, but Windows resolves the
        // same relative subkey names under it, so the precedence logic exercises identically.
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        struct Cleanup<'a>(&'a RegKey, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete_subkey_all(self.1);
            }
        }
        let native_key = r"Software\Blizzard Entertainment\World of Warcraft";
        let legacy_key = r"Software\WOW6432Node\Blizzard Entertainment\World of Warcraft";
        let _cleanup_native = Cleanup(&hkcu, native_key);
        let _cleanup_legacy = Cleanup(&hkcu, legacy_key);

        let (legacy, _) = hkcu.create_subkey(legacy_key).unwrap();
        legacy.set_value("InstallPath", &r"C:\Program Files (x86)\World of Warcraft").unwrap();

        // Only the WOW6432Node key exists: that's still found (the previous, pre-fix behaviour).
        assert_eq!(
            registry_install_path_in(RegKey::predef(HKEY_CURRENT_USER)),
            Some(PathBuf::from(r"C:\Program Files (x86)\World of Warcraft"))
        );

        // Once the native 64-bit key also exists, it wins: that's where a modern Agent actually writes.
        let (native, _) = hkcu.create_subkey(native_key).unwrap();
        native.set_value("InstallPath", &r"D:\Games\World of Warcraft").unwrap();
        assert_eq!(
            registry_install_path_in(RegKey::predef(HKEY_CURRENT_USER)),
            Some(PathBuf::from(r"D:\Games\World of Warcraft"))
        );
    }
}
