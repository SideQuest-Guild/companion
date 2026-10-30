//! Where the app keeps its files: the OS's per-user config and data folders for "SideQuest Companion" (via
//! `directories::ProjectDirs`). `settings.json` lives in the config dir; `state.json`, `history.json` and `logs/` in
//! the data dir.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "SideQuest Guild", "SideQuest Companion")
        .context("no home directory to keep settings in")
}

fn ensure(dir: PathBuf) -> Result<PathBuf> {
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

pub fn config_dir() -> Result<PathBuf> {
    ensure(project_dirs()?.config_dir().to_path_buf())
}

pub fn data_dir() -> Result<PathBuf> {
    ensure(project_dirs()?.data_dir().to_path_buf())
}

pub fn log_dir() -> Result<PathBuf> {
    ensure(data_dir()?.join("logs"))
}
