//! Self-updating the companion app itself: picking this platform's release asset, downloading and
//! verifying it, swapping it into place, and relaunching.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::release::{self, GithubRelease};

/// The release asset name for this platform (per the companion's release layout), or `None` if
/// this platform doesn't get a release build.
pub fn asset_name(version: &str) -> Option<String> {
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        Some(format!("sidequest-companion-{version}-windows-aarch64.zip"))
    }
    #[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
    {
        Some(format!("sidequest-companion-{version}-windows-x86_64.zip"))
    }
    #[cfg(target_os = "macos")]
    {
        Some(format!("sidequest-companion-{version}-macos-universal.zip"))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(format!("sidequest-companion-{version}-linux-x86_64.tar.gz"))
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "windows"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        let _ = version;
        None
    }
}

/// Whether this install can replace itself in place (the exe/bundle's directory is writable). The
/// UI should offer a plain download link instead when this is `false` (e.g. a root-owned
/// `/Applications` install or a distro package).
pub fn can_self_update() -> bool {
    match target_dir() {
        Ok(dir) => is_writable(&dir),
        Err(_) => false,
    }
}

fn is_writable(dir: &Path) -> bool {
    tempfile::Builder::new().prefix(".sqc-write-test-").tempfile_in(dir).is_ok()
}

fn target_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Ok(app_bundle_path()?
            .parent()
            .context("the app bundle has no parent directory")?
            .to_path_buf())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let exe = std::env::current_exe().context("locating the running executable")?;
        Ok(exe.parent().context("the executable has no parent directory")?.to_path_buf())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn app_bundle_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating the running executable")?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
        .context("not running from a .app bundle")
}

/// Download this platform's asset from `release` (verifying its checksum) and install it in
/// place. The caller should call [`relaunch`] and exit afterwards.
pub async fn download_and_install(
    client: &reqwest::Client,
    release: &GithubRelease,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<()> {
    let name =
        asset_name(&release.version).context("no release asset is built for this platform")?;
    let bytes = release::download_verified(client, release, &name, progress).await?;
    install_bytes(&bytes)
}

#[cfg(target_os = "macos")]
fn install_bytes(bytes: &[u8]) -> Result<()> {
    let app_path = app_bundle_path()?;
    let parent = app_path.parent().context("the app bundle has no parent directory")?;
    let staging = tempfile::tempdir_in(parent).context("creating a staging directory")?;
    extract_zip_tree(bytes, staging.path())?;
    let new_app = find_app_bundle(staging.path())?;

    let app_name = app_path.file_name().context("the app bundle has no name")?;
    let backup = parent.join(format!("{}.old", app_name.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&backup);

    std::fs::rename(&app_path, &backup).context("backing up the running app")?;
    match std::fs::rename(&new_app, &app_path) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::rename(&backup, &app_path);
            Err(err).context("installing the new app")
        }
    }
}

#[cfg(target_os = "macos")]
fn extract_zip_tree(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("opening the update zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("reading an update zip entry")?;
        let raw_name = String::from_utf8_lossy(file.name_raw()).into_owned();
        if file.is_symlink() {
            anyhow::bail!("update zip entry {raw_name} is a symlink; refusing to install");
        }
        let name = file
            .enclosed_name()
            .with_context(|| format!("update zip entry {raw_name} has an unsafe path"))?;
        let out_path = dest.join(&name);
        if file.is_dir() {
            std::fs::create_dir_all(&out_path)
                .with_context(|| format!("creating {}", out_path.display()))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mode = file.unix_mode();
        let mut out = std::fs::File::create(&out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("writing {}", out_path.display()))?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))
                .with_context(|| format!("setting permissions on {}", out_path.display()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn find_app_bundle(dir: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(dir).context("reading the staging directory")? {
        let path = entry.context("reading a staging directory entry")?.path();
        if path.extension().is_some_and(|ext| ext == "app") {
            return Ok(path);
        }
    }
    anyhow::bail!("update zip has no .app bundle")
}

#[cfg(target_os = "windows")]
fn install_bytes(bytes: &[u8]) -> Result<()> {
    let temp_dir = tempfile::tempdir().context("creating a temp directory")?;
    let new_exe = temp_dir.path().join("sidequest-companion.exe");
    extract_zip_member(bytes, "sidequest-companion.exe", &new_exe)?;
    self_replace::self_replace(&new_exe).context("replacing the running executable")?;
    let _ = std::fs::remove_file(&new_exe);
    Ok(())
}

/// The installer's Apps & features entry (see `packaging/windows/installer.nsi`).
#[cfg(target_os = "windows")]
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\SideQuestCompanion";

/// Keeps the version Apps & features shows in step with the running exe after a self-update, when this exe is
/// the one the installer put there. Copies run from elsewhere (an unzipped download) leave the entry alone.
#[cfg(target_os = "windows")]
pub fn record_installed_version() {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(UNINSTALL_KEY, KEY_READ | KEY_WRITE)
    else {
        return;
    };
    let Ok(location) = key.get_value::<String, _>("InstallLocation") else { return };
    let Some(exe_dir) =
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return;
    };
    let same_dir = exe_dir.to_string_lossy().trim_end_matches('\\').to_lowercase()
        == location.trim_end_matches('\\').to_lowercase();
    if same_dir && let Err(err) = key.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION")) {
        tracing::warn!("could not update the installed version shown in Apps & features: {err}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn record_installed_version() {}

#[cfg(target_os = "windows")]
fn extract_zip_member(bytes: &[u8], member_name: &str, dest: &Path) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("opening the update zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("reading an update zip entry")?;
        if file.is_symlink() {
            continue;
        }
        let Some(name) = file.enclosed_name() else { continue };
        if name.file_name().and_then(|n| n.to_str()) == Some(member_name) {
            let mut out = std::fs::File::create(dest)
                .with_context(|| format!("creating {}", dest.display()))?;
            std::io::copy(&mut file, &mut out)
                .with_context(|| format!("writing {}", dest.display()))?;
            return Ok(());
        }
    }
    anyhow::bail!("update zip has no {member_name}")
}

#[cfg(target_os = "linux")]
fn install_bytes(bytes: &[u8]) -> Result<()> {
    let temp_dir = tempfile::tempdir().context("creating a temp directory")?;
    let new_bin = temp_dir.path().join("sidequest-companion");
    extract_tar_gz_member(bytes, "sidequest-companion", &new_bin)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755))
            .context("marking the new binary executable")?;
    }
    self_replace::self_replace(&new_bin).context("replacing the running executable")?;
    let _ = std::fs::remove_file(&new_bin);
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_tar_gz_member(bytes: &[u8], member_name: &str, dest: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("reading the update archive")? {
        let mut entry = entry.context("reading an update archive entry")?;
        let path = entry.path().context("reading an update archive entry path")?.into_owned();
        if path.file_name().and_then(|n| n.to_str()) == Some(member_name) {
            let mut out = std::fs::File::create(dest)
                .with_context(|| format!("creating {}", dest.display()))?;
            std::io::copy(&mut entry, &mut out)
                .with_context(|| format!("writing {}", dest.display()))?;
            return Ok(());
        }
    }
    anyhow::bail!("update archive has no {member_name}")
}

/// Relaunch the (now-updated) app. The caller should exit immediately afterwards.
#[cfg(target_os = "macos")]
pub fn relaunch() -> Result<()> {
    let app = app_bundle_path()?;
    std::process::Command::new("open")
        .arg("-n")
        .arg(&app)
        .spawn()
        .context("relaunching the app")?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn relaunch() -> Result<()> {
    let exe = std::env::current_exe().context("locating the running executable")?;
    std::process::Command::new(exe).spawn().context("relaunching the app")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_matches_this_platform() {
        let name = asset_name("1.2.3").expect("this platform has a release asset in CI");
        assert!(name.starts_with("sidequest-companion-1.2.3-"), "{name}");
        #[cfg(target_os = "macos")]
        assert_eq!(name, "sidequest-companion-1.2.3-macos-universal.zip");
        #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
        assert_eq!(name, "sidequest-companion-1.2.3-windows-aarch64.zip");
        #[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
        assert_eq!(name, "sidequest-companion-1.2.3-windows-x86_64.zip");
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(name, "sidequest-companion-1.2.3-linux-x86_64.tar.gz");
    }

    #[cfg(unix)]
    fn running_as_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .is_some_and(|uid| uid.trim() == "0")
    }

    #[test]
    fn is_writable_detects_read_only_directories() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_writable(dir.path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
            // Root bypasses Unix permission bits, so this assertion only holds for non-root runs.
            if !running_as_root() {
                assert!(!is_writable(dir.path()));
            }
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}
