//! Launch-at-login, via `auto-launch`. On macOS this targets the `.app` bundle (Launch Agent mode,
//! which works unsigned) when running from one; everywhere else it targets the executable.

use anyhow::{Context, Result};
use auto_launch::AutoLaunchBuilder;
#[cfg(target_os = "linux")]
use auto_launch::LinuxLaunchMode;
#[cfg(target_os = "macos")]
use auto_launch::MacOSLaunchMode;

const APP_NAME: &str = "SideQuest Companion";
/// Login launches should start hidden in the tray; `CompanionUI` handles this flag.
const LAUNCH_ARGS: &[&str] = &["--hidden"];

fn launch_target() -> Result<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(app) = crate::self_update::app_bundle_path() {
            return Ok(app);
        }
        std::env::current_exe().context("locating the running executable")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::current_exe().context("locating the running executable")
    }
}

fn build() -> Result<auto_launch::AutoLaunch> {
    let target = launch_target()?;
    let app_path = target.to_str().context("the executable path is not valid UTF-8")?;
    let mut builder = AutoLaunchBuilder::new();
    builder.set_app_name(APP_NAME).set_app_path(app_path).set_args(LAUNCH_ARGS);
    #[cfg(target_os = "macos")]
    builder.set_macos_launch_mode(MacOSLaunchMode::LaunchAgent);
    #[cfg(target_os = "linux")]
    builder.set_linux_launch_mode(LinuxLaunchMode::XdgAutostart);
    builder.build().context("configuring launch-at-login")
}

pub fn set_launch_at_login(enabled: bool) -> Result<()> {
    let auto = build()?;
    if enabled {
        auto.enable().context("enabling launch at login")
    } else {
        auto.disable().context("disabling launch at login")
    }
}

pub fn is_enabled() -> Result<bool> {
    build()?.is_enabled().context("checking launch-at-login status")
}
