//! SideQuest Companion: watches the SideQuest addon's SavedVariables, uploads new exports to sidequest-guild.com,
//! stages the game import back for the addon, keeps the addon and the app up to date, and reminds you about raids.
//!
//! `engine` runs everything in the background; `ui` draws the window and the tray icon. They share only
//! [`state::Snapshot`] (engine writes, UI reads) and [`state::Command`] (UI to engine).

pub mod addon_install;
pub mod api;
pub mod autostart;
pub mod config;
pub mod engine;
pub mod history;
pub mod notifications;
pub mod paths;
pub mod release;
pub mod reminders;
pub mod secrets;
pub mod self_update;
pub mod state;
pub mod sync;
pub mod ui;
pub mod wow;
