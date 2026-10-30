//! Installs the addon zip from a [`crate::release::GithubRelease`] into a flavor's
//! `Interface/AddOns` folder. `install_zip` is zip-slip safe: every entry must resolve under
//! `SideQuest/`, with no absolute paths, `..`, or symlinks.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use zip::ZipArchive;

use crate::release::GithubRelease;

/// Download the addon zip from `release` (verifying its checksum) and install it into
/// `flavor_dir/Interface/AddOns`. Returns the installed `.toc` version.
pub async fn install_latest(
    client: &reqwest::Client,
    release: &GithubRelease,
    flavor_dir: &Path,
) -> Result<String> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".zip") && a.name != "SHA256SUMS.txt")
        .with_context(|| format!("release {} has no addon zip asset", release.tag))?;
    let bytes = crate::release::download_verified(client, release, &asset.name, &|_| {}).await?;
    let addons_dir = flavor_dir.join("Interface").join("AddOns");
    install_zip(&bytes, &addons_dir)?;
    read_installed_version(&addons_dir)
}

/// Extract an addon zip (root folder `SideQuest/` containing `SideQuest.toc`) into `addons_dir`.
/// Validates every entry before writing anything; keeps the previous `SideQuest/` as
/// `SideQuest.companion-backup` until the swap succeeds, and restores it on any failure.
pub fn install_zip(bytes: &[u8], addons_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(addons_dir)
        .with_context(|| format!("creating {}", addons_dir.display()))?;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("opening the addon zip")?;

    // Validate every entry before writing anything.
    let mut has_toc = false;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive.by_index(i).context("reading an addon zip entry")?;
        let raw_name = String::from_utf8_lossy(file.name_raw()).into_owned();
        if file.is_symlink() {
            anyhow::bail!("addon zip entry {raw_name} is a symlink; refusing to install");
        }
        let name = file
            .enclosed_name()
            .with_context(|| format!("addon zip entry {raw_name} has an unsafe path"))?;
        let mut components = name.components();
        let root =
            components.next().with_context(|| format!("addon zip entry {raw_name} has no path"))?;
        if root.as_os_str() != "SideQuest" {
            anyhow::bail!("addon zip entry {} is outside the SideQuest/ folder", name.display());
        }
        if !has_toc && name == Path::new("SideQuest/SideQuest.toc") {
            has_toc = true;
        }
        let is_dir = file.is_dir();
        entries.push((i, name, is_dir));
    }
    if !has_toc {
        anyhow::bail!("addon zip has no SideQuest/SideQuest.toc; refusing to install");
    }

    let staging = tempfile::tempdir_in(addons_dir).context("creating a staging directory")?;
    for (i, name, is_dir) in &entries {
        let dest = staging.path().join(name);
        if *is_dir {
            std::fs::create_dir_all(&dest)
                .with_context(|| format!("creating {}", dest.display()))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut file = archive.by_index(*i).context("reading an addon zip entry")?;
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("creating {}", dest.display()))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("writing {}", dest.display()))?;
    }

    let staged_addon = staging.path().join("SideQuest");
    let final_addon = addons_dir.join("SideQuest");
    let backup = addons_dir.join("SideQuest.companion-backup");
    let _ = std::fs::remove_dir_all(&backup);

    let had_existing = final_addon.exists();
    if had_existing {
        std::fs::rename(&final_addon, &backup).context("backing up the existing addon")?;
    }
    match std::fs::rename(&staged_addon, &final_addon) {
        Ok(()) => {
            if had_existing {
                let _ = std::fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(err) => {
            if had_existing {
                let _ = std::fs::rename(&backup, &final_addon);
            }
            Err(err).context("installing the new addon files")
        }
    }
}

fn read_installed_version(addons_dir: &Path) -> Result<String> {
    let toc = addons_dir.join("SideQuest").join("SideQuest.toc");
    let text =
        std::fs::read_to_string(&toc).with_context(|| format!("reading {}", toc.display()))?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("## Version:") {
            return Ok(rest.trim().to_owned());
        }
    }
    anyhow::bail!("{} has no \"## Version:\" line", toc.display())
}

#[cfg(test)]
mod tests {
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut writer, data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn install_zip_happy_path_reads_back_version() {
        let dir = tempfile::tempdir().unwrap();
        let toc = b"## Interface: 110000\n## Version: v0.4.1\n## Title: SideQuest\n";
        let zip =
            build_zip(&[("SideQuest/SideQuest.toc", toc), ("SideQuest/Core.lua", b"-- core")]);
        install_zip(&zip, dir.path()).unwrap();
        assert!(dir.path().join("SideQuest/Core.lua").exists());
        assert_eq!(read_installed_version(dir.path()).unwrap(), "v0.4.1");
    }

    #[test]
    fn install_zip_rejects_missing_toc() {
        let dir = tempfile::tempdir().unwrap();
        let zip = build_zip(&[("SideQuest/Core.lua", b"-- core")]);
        let err = install_zip(&zip, dir.path()).unwrap_err();
        assert!(err.to_string().contains("SideQuest.toc"), "{err}");
    }

    #[test]
    fn install_zip_rejects_path_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let zip = build_zip(&[
            ("SideQuest/SideQuest.toc", b"## Version: 1.0.0\n"),
            ("Other/evil.lua", b"evil"),
        ]);
        let err = install_zip(&zip, dir.path()).unwrap_err();
        assert!(err.to_string().contains("outside the SideQuest/ folder"), "{err}");
    }

    #[test]
    fn install_zip_rejects_zip_slip_traversal() {
        let dir = tempfile::tempdir().unwrap();
        // `enclosed_name()` rejects traversal entries entirely, so this must be a hard refusal.
        let zip = build_zip(&[
            ("SideQuest/SideQuest.toc", b"## Version: 1.0.0\n"),
            ("SideQuest/../../evil.lua", b"evil"),
        ]);
        let err = install_zip(&zip, dir.path()).unwrap_err();
        assert!(err.to_string().contains("unsafe path"), "{err}");
    }

    #[test]
    fn install_zip_rejects_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let zip = build_zip(&[
            ("SideQuest/SideQuest.toc", b"## Version: 1.0.0\n"),
            ("/etc/evil.lua", b"evil"),
        ]);
        let err = install_zip(&zip, dir.path()).unwrap_err();
        // `enclosed_name()` strips the leading `/`, so an absolute path lands outside `SideQuest/`
        // rather than tripping the "unsafe path" check directly; either way it must be refused.
        assert!(err.to_string().contains("outside the SideQuest/ folder"), "{err}");
    }

    #[test]
    fn install_zip_preserves_existing_install_when_zip_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let addon_dir = dir.path().join("SideQuest");
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::write(addon_dir.join("SideQuest.toc"), b"## Version: v0.1.0\n").unwrap();

        let bad_zip = build_zip(&[("Other/evil.lua", b"evil")]);
        let err = install_zip(&bad_zip, dir.path()).unwrap_err();
        assert!(err.to_string().contains("outside the SideQuest/ folder"), "{err}");
        // Validation fails before any rename, so the original install must be untouched.
        assert_eq!(read_installed_version(dir.path()).unwrap(), "v0.1.0");
    }

    #[cfg(unix)]
    #[test]
    fn install_zip_restores_backup_when_the_swap_rename_fails() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let addon_dir = dir.path().join("SideQuest");
        std::fs::create_dir_all(&addon_dir).unwrap();
        std::fs::write(addon_dir.join("SideQuest.toc"), b"## Version: v0.1.0\n").unwrap();

        let zip = build_zip(&[("SideQuest/SideQuest.toc", b"## Version: v0.2.0\n")]);

        // Make addons_dir read-only so the final rename (staged -> SideQuest) fails after the
        // existing install has already been backed up, forcing the restore path.
        let original_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = install_zip(&zip, dir.path());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(original_mode))
            .unwrap();

        assert!(result.is_err(), "expected the swap to fail under a read-only addons_dir");
        assert_eq!(read_installed_version(dir.path()).unwrap(), "v0.1.0");
        assert!(!dir.path().join("SideQuest.companion-backup").exists());
    }
}
